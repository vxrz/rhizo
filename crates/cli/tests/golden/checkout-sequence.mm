<map version="1.0.1">
<node ID="ID_root" TEXT="Checkout">
  <node ID="ID_customer" TEXT="Customer" POSITION="right">
    <arrowlink DESTINATION="ID_web" ENDARROW="Default" STARTARROW="None"/>
  </node>
  <node ID="ID_web" TEXT="Web app" POSITION="right">
    <arrowlink DESTINATION="ID_api" ENDARROW="Default" STARTARROW="None"/>
    <arrowlink DESTINATION="ID_customer" ENDARROW="Default" STARTARROW="None"/>
  </node>
  <node ID="ID_api" TEXT="Orders API" POSITION="right">
    <arrowlink DESTINATION="ID_pay" ENDARROW="Default" STARTARROW="None"/>
    <arrowlink DESTINATION="ID_api" ENDARROW="Default" STARTARROW="None"/>
    <arrowlink DESTINATION="ID_web" ENDARROW="Default" STARTARROW="None"/>
  </node>
  <node ID="ID_pay" TEXT="Payments" POSITION="right">
    <arrowlink DESTINATION="ID_api" ENDARROW="Default" STARTARROW="None"/>
  </node>
</node>
</map>
