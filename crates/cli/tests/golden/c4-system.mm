<map version="1.0.1">
<node ID="ID_root" TEXT="Online shop: context, containers, components">
  <node TEXT="System context" POSITION="right">
    <node ID="ID_customer" TEXT="Customer" BACKGROUND_COLOR="#1d4ed8">
      <richcontent TYPE="NOTE"><html><head/><body><p>Browses the catalog, orders and pays</p></body></html></richcontent>
      <arrowlink DESTINATION="ID_web" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_payments" TEXT="Payment provider">
    </node>
    <node ID="ID_email" TEXT="Email service">
    </node>
  </node>
  <node TEXT="Online shop" POSITION="right">
    <node ID="ID_web" TEXT="Web shop">
      <richcontent TYPE="NOTE"><html><head/><body><p>Single-page app in the browser</p></body></html></richcontent>
      <arrowlink DESTINATION="ID_orders_ctl" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_orders_db" TEXT="Orders DB">
    </node>
  </node>
  <node TEXT="API" POSITION="right">
    <node ID="ID_orders_ctl" TEXT="Orders controller">
      <arrowlink DESTINATION="ID_checkout" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_checkout" TEXT="Checkout service">
      <arrowlink DESTINATION="ID_repo" ENDARROW="Default" STARTARROW="None"/>
      <arrowlink DESTINATION="ID_payments" ENDARROW="Default" STARTARROW="None"/>
      <arrowlink DESTINATION="ID_email" ENDARROW="Default" STARTARROW="None"/>
    </node>
    <node ID="ID_repo" TEXT="Order repository">
      <arrowlink DESTINATION="ID_orders_db" ENDARROW="Default" STARTARROW="None"/>
    </node>
  </node>
</node>
</map>
