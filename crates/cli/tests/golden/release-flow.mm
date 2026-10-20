<map version="1.0.1">
<node ID="ID_root" TEXT="Release pipeline">
  <node TEXT="Release pipeline" POSITION="right">
    <node ID="ID_merge_to_main" TEXT="Merge to main">
      <arrowlink DESTINATION="ID_build_artifacts" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_build_artifacts" TEXT="Build artifacts">
      <arrowlink DESTINATION="ID_unit_tests" ENDARROW="Default" STARTARROW="None"/>
      <arrowlink DESTINATION="ID_integration_tests" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_unit_tests" TEXT="Unit tests">
      <arrowlink DESTINATION="ID_green" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_integration_tests" TEXT="Integration tests">
      <arrowlink DESTINATION="ID_green" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_green" TEXT="All green?" BACKGROUND_COLOR="#fef3c7">
      <arrowlink DESTINATION="ID_publish_crates" ENDARROW="Default" STARTARROW="None"/>
      <arrowlink DESTINATION="ID_open_incident" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_publish_crates" TEXT="Publish crates">
      <arrowlink DESTINATION="ID_notify_team" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_notify_team" TEXT="Notify team">
    </node>
    <node ID="ID_open_incident" TEXT="Open incident">
      <arrowlink DESTINATION="ID_build_artifacts" ENDARROW="Default" STARTARROW="None"/>
    </node>
  </node>
</node>
</map>
