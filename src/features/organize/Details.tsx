// What a small card leaves out at first (the pet's popup, #37): its
// explanations, kept one click away in a closed "Details". Nothing is
// dropped; it is only folded.

import type { ReactNode } from "react";
import "./compact.css";

export function Details(props: { label?: string; children: ReactNode }) {
  return (
    <details className="compact-details">
      <summary>{props.label ?? "Details"}</summary>
      <div className="compact-details-body">{props.children}</div>
    </details>
  );
}
