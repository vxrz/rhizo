//! Running OpenTofu offline: in a temporary copy, against an empty local
//! state, with every provider mocked and none of the caller's cloud config.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::Value;

use crate::Error;
use crate::hcl::{self, Block};
use crate::mocks;
use crate::providers;

/// A temporary folder, removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Result<TempDir, Error> {
        use std::sync::atomic::AtomicU32;
        static N: AtomicU32 = AtomicU32::new(0);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("rhizo-iac-{}-{nanos}-{n}", std::process::id()));
        // Not recursive: fails rather than reuse a folder someone else made.
        let mut dir = std::fs::DirBuilder::new();
        // Private: the copy holds variable files, which may be secret.
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
        dir.create(&path).map_err(|e| Error::Io(format!("cannot create {}: {e}", path.display())))?;
        Ok(TempDir(path))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One import or verification: its temporary folders and plugin cache.
pub struct Run {
    root: TempDir,
    bin: PathBuf,
    cache: PathBuf,
    empty: PathBuf,
    cancel: Option<Arc<AtomicBool>>,
    /// The providers this run may install ([`providers::ALLOWED`] and the person's).
    allowed: Vec<String>,
    /// rhizo's CLI configuration for `tofu`, which installs only `allowed`.
    config: PathBuf,
}

/// Environment variables passed through to `tofu`: how to reach the
/// registry, nothing about cloud accounts. Not `TF_CLI_CONFIG_FILE` (the run
/// sets its own) nor `TF_CLI_ARGS*`, which could add arguments.
const KEEP: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "TMPDIR",
    "TEMP",
    "TMP",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

impl Run {
    pub fn new(opts: &crate::Options) -> Result<Run, Error> {
        let root = TempDir::new()?;
        let io = |e: std::io::Error| Error::Io(e.to_string());
        // Providers are large: both plans of a run share one download, or
        // the caller's cache.
        let cache = match (&opts.plugin_cache, std::env::var_os("TF_PLUGIN_CACHE_DIR")) {
            (Some(c), _) => c.clone(),
            (None, Some(c)) if !c.is_empty() => PathBuf::from(c),
            _ => root.path().join("plugins"),
        };
        std::fs::create_dir_all(&cache).map_err(io)?;
        let empty = root.path().join("empty");
        std::fs::write(&empty, "").map_err(io)?;
        let mut allowed: Vec<String> = providers::ALLOWED.iter().map(|s| s.to_string()).collect();
        for a in &opts.allow_providers {
            allowed.push(providers::parse_allowed(a).map_err(Error::Io)?);
        }
        let config = root.path().join("rhizo.tofurc");
        std::fs::write(&config, providers::cli_config(&allowed)).map_err(io)?;
        Ok(Run { root, bin: opts.tofu.clone(), cache, empty, cancel: opts.cancel.clone(), allowed, config })
    }

    /// A new empty working folder.
    pub fn workdir(&self, name: &str) -> Result<PathBuf, Error> {
        let dir = self.root.path().join(name);
        std::fs::create_dir(&dir).map_err(|e| Error::Io(format!("cannot create {}: {e}", dir.display())))?;
        Ok(dir)
    }

    fn output(&self, dir: &Path, args: &[&str]) -> Result<Output, Error> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(args).current_dir(dir).env_clear().stdin(Stdio::null());
        for k in KEEP {
            if let Some(v) = std::env::var_os(k) {
                cmd.env(k, v);
            }
        }
        // With its own config file set, `tofu` reads no other: not ~/.tofurc,
        // ~/.terraformrc nor ~/.terraform.d (OpenTofu's cliconfig.LoadConfig).
        cmd.env("TF_CLI_CONFIG_FILE", &self.config)
            .env("TF_PLUGIN_CACHE_DIR", &self.cache)
            .env("TF_IN_AUTOMATION", "1")
            .env("TF_INPUT", "0")
            .env("CHECKPOINT_DISABLE", "1")
            // Providers are mocked; still, no way to find real credentials.
            .env("AWS_EC2_METADATA_DISABLED", "true")
            .env("AWS_CONFIG_FILE", &self.empty)
            .env("AWS_SHARED_CREDENTIALS_FILE", &self.empty)
            // A `gcs::` module source would otherwise use ~/.config/gcloud's credentials.
            .env("GOOGLE_APPLICATION_CREDENTIALS", &self.empty);
        let cannot = |e: std::io::Error| Error::Io(format!("cannot run {}: {e}", self.bin.display()));
        let Some(cancel) = &self.cancel else { return cmd.output().map_err(cannot) };
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        // Cancellable: the pipes are drained on threads while the child is polled.
        let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(cannot)?;
        let drain = |r: Option<Box<dyn Read + Send>>| {
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                if let Some(mut r) = r {
                    let _ = r.read_to_end(&mut buf);
                }
                buf
            })
        };
        let out = drain(child.stdout.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
        let err = drain(child.stderr.take().map(|r| Box::new(r) as Box<dyn Read + Send>));
        let status = loop {
            if let Some(s) = child.try_wait().map_err(cannot)? {
                break s;
            }
            if cancel.load(Ordering::Relaxed) {
                // shortcut: kills `tofu` only; its provider plugins exit when their parent's pipe closes.
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        Ok(Output { status, stdout: out.join().unwrap_or_default(), stderr: err.join().unwrap_or_default() })
    }

    fn tofu(&self, dir: &Path, what: &str, args: &[&str]) -> Result<String, Error> {
        let out = self.output(dir, args)?;
        if !out.status.success() {
            let text = String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout);
            return Err(Error::Tofu { step: what.into(), output: text.trim().to_string() });
        }
        String::from_utf8(out.stdout).map_err(|_| Error::Io(format!("{what}: output is not UTF-8")))
    }

    /// Plans the root module in `dir` (a copy) against an empty state with
    /// every provider mocked, and returns the plan as `tofu show -json`
    /// shapes it. `blocks` are its blocks, for the provider aliases.
    pub fn plan(&self, dir: &Path, blocks: &[Block], what: &str) -> Result<Value, Error> {
        self.plan_with_schema(dir, blocks, what).map(|(plan, _)| plan)
    }

    /// [`Run::plan`], with the providers' schema (`tofu providers schema -json`).
    pub fn plan_with_schema(&self, dir: &Path, blocks: &[Block], what: &str) -> Result<(Value, Value), Error> {
        let io = |e: std::io::Error| Error::Io(e.to_string());
        for e in std::fs::read_dir(dir).map_err(io)? {
            let path = e.map_err(io)?.path();
            let tf = path.extension().is_some_and(|x| x == "tf" || x == "tofu");
            if let Some(text) = tf.then(|| std::fs::read_to_string(&path).ok()).flatten()
                && let Some(cut) = hcl::without_imports(&text)
            {
                std::fs::write(&path, cut).map_err(io)?;
            }
        }
        std::fs::write(dir.join("zz_rhizo_offline_override.tf"), OVERRIDE).map_err(io)?;
        // Refused by name before anything is installed: first what the root
        // module and its lock file name, then (modules are code, not
        // programs) what the modules name once `tofu get` fetched them.
        providers::check(dir, &self.allowed)?;
        if blocks.iter().any(|b| b.kind == "module") {
            self.tofu(dir, &format!("tofu get ({what})"), &["get", "-no-color"])?;
            providers::check(dir, &self.allowed)?;
        }
        self.tofu(dir, &format!("tofu init ({what})"), &["init", "-input=false", "-no-color"])?;
        let schema = self.tofu(dir, &format!("tofu providers schema ({what})"), &["providers", "schema", "-json"])?;
        let schema: Value =
            serde_json::from_str(&schema).map_err(|e| Error::Io(format!("tofu providers schema ({what}): {e}")))?;
        // A provider only a module requires (`Azure/azapi` in a Verified Module the root does
        // not declare) needs a root local name to be mocked: the override gives it one.
        let declared = |ty: &str| {
            blocks.iter().filter(|b| b.kind == "terraform").any(|b| {
                b.body
                    .get("required_providers")
                    .and_then(Value::as_array)
                    .is_some_and(|r| r.iter().any(|x| x.get(ty).is_some()))
            })
        };
        let extra: String = schema["provider_schemas"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(address, _)| match address.split('/').collect::<Vec<_>>()[..] {
                [_, ns, ty] if ns != "hashicorp" && ns != "builtin" && !declared(ty) => {
                    Some(format!("    {ty} = {{\n      source = \"{ns}/{ty}\"\n    }}\n"))
                }
                _ => None,
            })
            .collect();
        if !extra.is_empty() {
            let with =
                OVERRIDE.replacen("terraform {\n", &format!("terraform {{\n  required_providers {{\n{extra}  }}\n"), 1);
            std::fs::write(dir.join("zz_rhizo_offline_override.tf"), with).map_err(io)?;
        }
        std::fs::write(
            dir.join(mocks::TEST_FILE),
            mocks::test_file(
                &schema,
                blocks,
                &providers::local_names(dir),
                &[
                    mocks::azapi_overrides(dir),
                    mocks::google_overrides(dir),
                    mocks::remote_state_overrides(dir).map_err(Error::Io)?,
                ]
                .concat(),
            ),
        )
        .map_err(io)?;
        // Only rhizo's test file: the module's own tests may apply real infrastructure.
        let filter = format!("-filter={}", mocks::TEST_FILE);
        let out = self.output(dir, &["test", &filter, "-json", "-verbose", "-no-color"])?;
        let plan = mocks::plan_of(&String::from_utf8_lossy(&out.stdout)).map_err(|output| Error::Tofu {
            step: format!("tofu plan ({what})"),
            output: if output.contains("no plan") {
                format!("{output}\n{}", String::from_utf8_lossy(&out.stderr).trim())
            } else {
                output
            },
        })?;
        Ok((plan, schema))
    }
}

/// An override file that keeps state local, so `init` touches no backend.
const OVERRIDE: &str = "# Written by rhizo for an offline plan; never part of a diagram or bundle.\nterraform {\n  backend \"local\" {\n    path = \"rhizo.tfstate\"\n  }\n}\n";

/// Copies the root module in `from` to `to` for planning: files and
/// subfolders, without hidden tool folders, state, plans or symlinks.
pub fn copy_tree(from: &Path, to: &Path) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Io(format!("copying {}: {e}", from.display()));
    for e in std::fs::read_dir(from).map_err(io)? {
        let e = e.map_err(io)?;
        let name = e.file_name().to_string_lossy().into_owned();
        let ty = e.file_type().map_err(io)?;
        if (name.starts_with('.') && name != hcl::LOCK) || name.contains(".tfstate") || name.ends_with(".tfplan") {
            continue;
        }
        if ty.is_dir() {
            std::fs::create_dir(to.join(&name)).map_err(io)?;
            copy_tree(&e.path(), &to.join(&name))?;
        } else if ty.is_file() {
            std::fs::copy(e.path(), to.join(&name)).map_err(io)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn temporary_folders_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        assert_eq!(std::fs::metadata(tmp.path()).unwrap().permissions().mode() & 0o777, 0o700);
    }

    /// `terraform_remote_state` (OpenTofu's built-in provider, not mocked)
    /// is overridden: real, it fetches any URL or reads any local file, and
    /// its address can carry `file()` contents out.
    #[test]
    #[ignore = "needs tofu; run with --ignored"]
    fn remote_state_reads_nothing() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("code");
        std::fs::create_dir_all(&dir).unwrap();
        let outside = tmp.path().join("outside.tfstate");
        let state = r#"{"version":4,"serial":1,"lineage":"a","outputs":{"pw":{"value":"SECRET","type":"string"}},"resources":[]}"#;
        std::fs::write(&outside, state).unwrap();
        // Counts every connection, and drops it.
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/${{base64encode(\"secret\")}}", server.local_addr().unwrap());
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            server.incoming().for_each(|c| {
                counted.fetch_add(1, Ordering::SeqCst);
                drop(c);
            })
        });
        // A precondition fails if the local state is read; one in a `check`
        // block (a scoped data source) only connects.
        let main = format!(
            "data \"terraform_remote_state\" \"file\" {{\n  backend = \"local\"\n  config = {{ path = {:?} }}\n}}\nresource \"terraform_data\" \"t\" {{\n  lifecycle {{\n    precondition {{\n      condition     = data.terraform_remote_state.file.outputs.pw == null\n      error_message = \"read\"\n    }}\n  }}\n}}\ncheck \"c\" {{\n  data \"terraform_remote_state\" \"net\" {{\n    backend = \"http\"\n    config = {{ address = \"{url}\" }}\n  }}\n  assert {{\n    condition     = data.terraform_remote_state.net.outputs != null\n    error_message = \"x\"\n  }}\n}}\n",
            outside.display().to_string()
        );
        std::fs::write(dir.join("main.tf"), &main).unwrap();
        let run = Run::new(&crate::Options::default()).unwrap();
        let blocks = hcl::parse("main.tf", &main).unwrap();
        if let Err(e) = run.plan(&dir, &blocks, "remote state") {
            panic!("remote state was read: {e}");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0, "remote state was fetched over the network");
    }

    /// A provider only a local module requires is refused after `tofu get`
    /// fetched the module, before `tofu init` downloads the provider.
    #[test]
    #[ignore = "needs tofu; run with --ignored"]
    fn a_local_modules_provider_is_refused_before_download() {
        let tmp = TempDir::new().unwrap();
        let (dir, cache) = (tmp.path().join("code"), tmp.path().join("cache"));
        std::fs::create_dir_all(dir.join("m")).unwrap();
        let main = "module \"m\" {\n  source = \"./m\"\n}\n";
        std::fs::write(dir.join("main.tf"), main).unwrap();
        let m = "terraform {\n  required_providers {\n    github = { source = \"integrations/github\" }\n  }\n}\n";
        std::fs::write(dir.join("m/main.tf"), m).unwrap();
        let run = Run::new(&crate::Options { plugin_cache: Some(cache.clone()), ..Default::default() }).unwrap();
        let blocks = hcl::parse("main.tf", main).unwrap();
        match run.plan(&dir, &blocks, "local module") {
            Err(Error::NotAllowed(found)) => assert_eq!(
                found,
                [(
                    "registry.opentofu.org/integrations/github".to_string(),
                    "module.m (main.tf, source \"./m\")".to_string()
                )]
            ),
            other => panic!("not refused: {other:?}"),
        }
        assert!(!cache.join("registry.opentofu.org/integrations").exists());
    }
}
