// "Now" (#37, the student's choice 10/14): the one thing the user is doing
// on a page, above its groups. Organize puts the file open in its flow
// here. It only frames the card; what the card shows and does is its own.

import type { ReactNode } from "react";
import "./groups.css";

export function Now(props: { children: ReactNode; label?: string }) {
  return (
    <div className="now">
      <p className="now-label">{props.label ?? "Now"}</p>
      {props.children}
    </div>
  );
}
