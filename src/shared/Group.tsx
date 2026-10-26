// Groups of rows on a page (#37, the student's choice B, 10/14): each group
// has a title, its count and its rows, in Find's row style. The page puts
// what waits for the user in the first group ("Needs you"): that group has
// an amber count and is always open. The others ("In progress", "Finished")
// can be closed; a closed group keeps its rows mounted (hidden), so what
// they hold stays, and may say in a few words what it holds.
//
// Long groups wrap their list in ShowMore, with `keep` for rows that wait
// for the user, so those never go behind "Show more". Content the user must
// see in full before confirming (a move's preview, a consent text, a list
// with checkboxes) is never put in a closed group or behind Show more.

import { useState, type ReactNode } from "react";
import { toneClass, type Tone } from "./status";
import "../features/find/find.css";
import "./groups.css";

export function Group(props: {
  /** The section's id; the title is `${id}-title`, the rows `${id}-rows`. */
  id: string;
  title: string;
  count: number;
  /** Waits for the user: amber count, always open. */
  attention?: boolean;
  /** Open at first (default true). Ignored with `attention`. */
  defaultOpen?: boolean;
  /** Open now whatever the user chose, e.g. to bring a row in it into view. */
  forceOpen?: boolean;
  /** A few words on what it holds, after the title while it is closed. */
  summary?: ReactNode;
  /** On the title line's right while it is open, e.g. Clear finished. */
  action?: ReactNode;
  /** A quiet line under the title while it is open. */
  note?: ReactNode;
  children: ReactNode;
}) {
  const { id, attention = false } = props;
  const [chosen, setChosen] = useState(props.defaultOpen ?? true);
  const open = attention || props.forceOpen === true || chosen;
  const count = (
    <span className={attention ? "group-count group-count-attention" : "group-count"}>{props.count}</span>
  );
  return (
    <section id={id} className="group" aria-labelledby={`${id}-title`}>
      <div className="group-head">
        <h2 id={`${id}-title`} className="group-title">
          {attention ? (
            <>
              {props.title} {count}
            </>
          ) : (
            <button
              type="button"
              className="group-toggle"
              aria-expanded={open}
              aria-controls={`${id}-rows`}
              onClick={() => setChosen(!open)}
            >
              <span className="group-chevron" aria-hidden="true" />
              {props.title} {count}
            </button>
          )}
        </h2>
        {!open && props.summary && <span className="group-summary">{props.summary}</span>}
        {open && props.action && <span className="group-action">{props.action}</span>}
      </div>
      <div id={`${id}-rows`} hidden={!open}>
        {props.note && <p className="group-note">{props.note}</p>}
        {props.children}
      </div>
    </section>
  );
}

/** A group's rows: one white box, a hairline between rows (Find's rows). */
export function Rows(props: { children: ReactNode }) {
  return (
    <ul className="find-rows" role="list">
      {props.children}
    </ul>
  );
}

/** One row: an icon, the name, its lines and status, then its buttons on
 * the right (one primary button per screen at most, chosen by the page).
 * `attention`: this row waits for the user (an amber edge). `children`
 * go under the text, e.g. a question answered in place. */
export function Row(props: {
  icon?: ReactNode;
  name: ReactNode;
  lines?: ReactNode;
  status?: ReactNode;
  actions?: ReactNode;
  attention?: boolean;
  children?: ReactNode;
}) {
  return (
    <li className={props.attention ? "find-row group-row-attention" : "find-row"}>
      {props.icon}
      <div className="find-row-text">
        <div className="find-row-name">{props.name}</div>
        {props.lines}
        {props.status}
        {props.children}
      </div>
      {props.actions && <div className="find-row-actions">{props.actions}</div>}
    </li>
  );
}

/** A row's status: the badge (status.ts tones), when, and an optional
 * short note with its full text in the tooltip. */
export function StatusLine(props: {
  badge: { label: string; tone: Tone };
  when?: string;
  note?: { short: string; full: string } | null;
}) {
  return (
    <div className="status-line">
      <span className={`badge ${toneClass(props.badge.tone)}`}>{props.badge.label}</span>
      {props.when && <span>{props.when}</span>}
      {props.note && <span title={props.note.full}>{props.note.short}</span>}
    </div>
  );
}
