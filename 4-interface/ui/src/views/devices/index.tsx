// Devices (spec 6.3). The router sends /devices, /devices/add and /devices/:id here; this view branches on the route.
import { useLocation, useParams } from "wouter";
import AddDevice from "./add";
import DeviceDetail from "./detail";
import DeviceList from "./list";

export default function DevicesView() {
  const [path] = useLocation();
  const { id } = useParams<{ id?: string }>();
  if (path === "/devices/add") return <AddDevice />;
  if (id) return <DeviceDetail key={id} id={id} />;
  return <DeviceList />;
}
