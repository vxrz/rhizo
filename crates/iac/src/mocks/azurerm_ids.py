"""Regenerates azurerm_ids.tsv from the azurerm provider docs.

  git clone --depth 1 --branch vX.Y.Z --filter=blob:none --sparse \
      https://github.com/hashicorp/terraform-provider-azurerm azurerm
  git -C azurerm sparse-checkout set website/docs/r
  python3 azurerm_ids.py azurerm/website/docs/r vX.Y.Z > azurerm_ids.tsv
"""
import glob, os, re, sys

docs, version = sys.argv[1], sys.argv[2]
ids = {}
for f in sorted(glob.glob(os.path.join(docs, "*.html.markdown"))):
    m = re.search(r"(?:terraform|tofu) import\s+(azurerm_\w+)\.\S+\s+(\S.*)", open(f).read())
    if not m:
        continue
    kind, example = m.group(1), m.group(2).strip().strip("\"'`")
    if (example.startswith("/") or example.startswith("https://")) and " " not in example:
        ids.setdefault(kind, example)
print(f"# azurerm resource type -> example ID, from the Import sections of terraform-provider-azurerm {version} docs (website/docs/r, MPL-2.0).")
print("# Regenerate with azurerm_ids.py when the pinned azurerm version changes; used for offline L1 mocks only.")
for kind in sorted(ids):
    print(f"{kind}\t{ids[kind]}")
