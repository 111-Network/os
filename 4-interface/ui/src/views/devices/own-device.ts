// Which companion record is this browser? `GET /auth/session` answers with `device_id`: the caller's own
// paired session, or null when the caller is not a paired companion (loopback, unpaired). Older daemons
// leave the field out. The Devices views use it to say "this browser" and to give a definite Forget warning.
import { useEffect, useState } from "react";
import { getAdapter } from "../../lib/api";

/** string: this browser's record id. null: the node says this is not a companion session. undefined: not known (yet). */
export type OwnDeviceId = string | null | undefined;

export function useOwnDeviceId(): OwnDeviceId {
  const [id, setId] = useState<OwnDeviceId>(undefined);
  useEffect(() => {
    let alive = true;
    getAdapter()
      .then((a) => a.auth.session())
      .then((s) => {
        if (alive) setId(s.device_id);
      })
      .catch(() => undefined); // unknown stays unknown: the hedged warning is the safe text
    return () => {
      alive = false;
    };
  }, []);
  return id;
}
