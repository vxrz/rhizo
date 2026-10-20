<map version="1.0.1">
<node ID="ID_root" TEXT="Golden mix">
  <node TEXT="main" POSITION="right">
    <node ID="ID_launch" TEXT="Launch">
      <node ID="ID_users" TEXT="Users">
        <node ID="ID_personas" TEXT="Personas">
        </node>
        <node ID="ID_jobs" TEXT="Jobs">
        </node>
      </node>
      <node ID="ID_pricing" TEXT="Pricing" BACKGROUND_COLOR="#fee2e2" LINK="https://example.com/pricing">
        <richcontent TYPE="NOTE"><html><head/><body><p>Needs a decision</p></body></html></richcontent>
        <arrowlink DESTINATION="ID_check" ENDARROW="Default" STARTARROW="None"/>
      </node>
      <node ID="ID_multi_line_label" TEXT="Multi&#10;line label">
      </node>
    </node>
  </node>
  <node TEXT="Release" POSITION="right">
    <node ID="ID_build" TEXT="Build">
      <arrowlink DESTINATION="ID_check" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_check" TEXT="Tests pass?">
      <arrowlink DESTINATION="ID_ship" ENDARROW="Default" STARTARROW="None"/>
      <arrowlink DESTINATION="ID_build" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_ship" TEXT="Ship" BACKGROUND_COLOR="#dcfce7">
    </node>
  </node>
  <node TEXT="Board" POSITION="right">
    <node ID="ID_idea" TEXT="Idea">
      <arrowlink DESTINATION="ID_question" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_question" TEXT="Question?">
    </node>
  </node>
</node>
</map>
