// Settings (UI redo, 10/13): the folders Traeva may organize, the model and
// what may be sent to it, text recognition for scanned PDFs and images, and
// organizing dropped files automatically. They were cards on the Organize page, which
// now shows only the files to organize. Nothing here is new: the same
// cards, the same commands, the same rules.
//
// In groups (#37, the student's choice B, 10/14): what needs the user
// first and always open, then what is getting ready, then what is set up.
// The groups are worked out when the page opens. While it stays open, a
// card that comes to need the user moves to Needs you at once (a card's
// change, the window coming back, text recognition failing; R3 P2-1); a
// card that got better stays where it is until the next visit, so it
// never jumps away under the user's hands.

import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { getAutoDropSettings } from "../autodrop/api";
import { AutoDropCard } from "../autodrop/AutoDropCard";
import { getAppState, getOcrState } from "../organize/api";
import { ModelSettingsCard } from "../organize/ModelSettingsCard";
import { OcrStateLine } from "../organize/OcrStateLine";
import { RootsCard } from "../organize/RootsCard";
import { ErrorDetails } from "../organize/shared";
import { isAppError, type AppError, type AppState, type OcrState } from "../organize/types";
import { Group } from "../../shared/Group";
import {
  allSetSummary,
  settingsGroups,
  worsened,
  type SettingsFacts,
  type SettingsGroups,
  type SettingsPart,
} from "./groups";

type PageState =
  | { kind: "loading" }
  | { kind: "loaded"; app: AppState }
  | { kind: "failed"; error: AppError | null; detail: string };

/** `active`: the Settings page is showing; it reads Traeva's state again
 * each time, since folders and the model also change elsewhere. */
/** `revealAutomatic`: bumped when the pet's popup asks for the automatic
 * mode's card ("Turn on automatic mode…"): its group opens and the card
 * comes into view, until the page is left. */
export function SettingsPage({ active, revealAutomatic = 0 }: { active: boolean; revealAutomatic?: number }) {
  const [page, setPage] = useState<PageState>({ kind: "loading" });
  // Counts the reads, so cards that show saved settings read them again.
  const [pageKey, setPageKey] = useState(0);
  // Where each card goes: worked out when the page opens, then only moved
  // towards Needs you (groups.ts, `worsened`).
  const [groups, setGroups] = useState<SettingsGroups | null>(null);
  // What the groups were last worked out from.
  const facts = useRef<SettingsFacts | null>(null);
  // Each card is rendered once, into its own element, and its group shows
  // that element. A card that moves to Needs you moves as that element, so
  // it is not made again and keeps what the user typed and did not save
  // (R3's P2 on #292).
  const nodes = useRef<Record<SettingsPart, HTMLDivElement> | null>(null);
  if (nodes.current === null) {
    nodes.current = {
      folders: document.createElement("div"),
      model: document.createElement("div"),
      recognition: document.createElement("div"),
      automatic: document.createElement("div"),
    };
  }
  const cardNodes = nodes.current;
  const place = useCallback((next: SettingsFacts, regroup: boolean) => {
    facts.current = next;
    const now = settingsGroups(next);
    setGroups((shown) => (regroup || shown === null ? now : worsened(shown, now)));
  }, []);

  // `regroup`: the page was opened. Otherwise (a card's change, the window
  // coming back) only a new problem moves a card.
  const read = useCallback(
    (regroup: boolean) => {
      getAppState()
        .then(async (app) => {
          // Read only to group the cards; each card reads and shows its own.
          const [recognition, automatic] = await Promise.all([
            getOcrState().catch(() => null),
            getAutoDropSettings().catch(() => null),
          ]);
          place({ roots: app.roots, model: app.model, recognition, automatic }, regroup);
          setPage({ kind: "loaded", app });
          setPageKey((key) => key + 1);
        })
        .catch((reason: unknown) =>
          setPage({ kind: "failed", error: isAppError(reason) ? reason : null, detail: String(reason) }),
        );
    },
    [place],
  );
  const changed = useCallback(() => read(false), [read]);
  // Text recognition can fail while the page is open (its line reads again).
  const recognized = useCallback(
    (recognition: OcrState) => {
      const current = facts.current;
      if (current !== null && current.recognition !== recognition) place({ ...current, recognition }, false);
    },
    [place],
  );

  useEffect(() => {
    if (active) read(true);
  }, [active, read]);
  // The automatic mode's card, asked for from the pet's popup: open its
  // group, then bring it into view once the page has read the settings.
  const [openAutomatic, setOpenAutomatic] = useState(false);
  const revealing = useRef(false);
  useEffect(() => {
    if (revealAutomatic === 0) return;
    revealing.current = true;
    setOpenAutomatic(true);
    read(true);
  }, [revealAutomatic, read]);
  useEffect(() => {
    if (!active) setOpenAutomatic(false);
  }, [active]);
  useEffect(() => {
    if (!revealing.current || pageKey === 0) return;
    revealing.current = false;
    const card = document.getElementById("settings-automatic");
    card?.scrollIntoView({ block: "start" });
    card?.focus({ preventScroll: true });
  }, [pageKey]);
  // A folder moved or removed in Finder shows when the window comes back.
  useEffect(() => {
    if (!active) return;
    window.addEventListener("focus", changed);
    return () => window.removeEventListener("focus", changed);
  }, [active, changed]);

  // The cards, by part (each shown through its own element, see `nodes`).
  const cards: Record<SettingsPart, ReactNode> =
    page.kind === "loaded"
      ? {
          folders: <RootsCard roots={page.app.roots} onChanged={changed} />,
          model: <ModelSettingsCard onChanged={changed} />,
          recognition: (
            <section className="card">
              <h2>Text recognition</h2>
              <OcrStateLine pageKey={pageKey} onState={recognized} />
            </section>
          ),
          automatic: (
            <div id="settings-automatic" className="settings-anchor" tabIndex={-1}>
              <AutoDropCard
                roots={page.app.roots.filter((root) => root.status === "active")}
                modelReady={page.app.model === "configured"}
                pageKey={pageKey}
              />
            </div>
          ),
        }
      : { folders: null, model: null, recognition: null, automatic: null };

  return (
    <main className="page">
      <header className="page-header">
        <h1>Settings</h1>
      </header>

      {page.kind === "loading" && <p className="muted">Checking storage…</p>}

      {page.kind === "failed" && (
        <section className="card card-error">
          <h2>Could not read Traeva's state</h2>
          {page.error ? <ErrorDetails error={page.error} /> : <p>{page.detail}</p>}
        </section>
      )}

      {page.kind === "loaded" && page.app.storage.status === "blocked" && (
        <section className="card card-error">
          <h2>Storage blocked</h2>
          <p>Traeva will not change any setting until this is fixed. Your files were not touched.</p>
          <ErrorDetails error={page.app.storage.error} />
        </section>
      )}

      {page.kind === "loaded" && page.app.storage.status === "ready" && groups !== null && (
        <>
          <SettingsInGroups groups={groups} open={openAutomatic ? "automatic" : null} nodes={cardNodes} />
          {placed(groups).map((part) => createPortal(cards[part], cardNodes[part], part))}
        </>
      )}
    </main>
  );
}

/** The cards in their groups. Every group is open at first; when nothing
 * needs the user and nothing is getting ready, All set is one closed line
 * that opens to the cards (A, 10/14). A card's consent text is never cut:
 * it shows in full whenever its card shows. */
function SettingsInGroups(props: {
  groups: SettingsGroups;
  /** A part asked for from outside (the popup): its group is open. */
  open: SettingsPart | null;
  /** Each part's element, which holds its card. */
  nodes: Record<SettingsPart, HTMLDivElement>;
}) {
  const { groups, open, nodes } = props;
  const holds = (list: SettingsPart[]) => open !== null && list.includes(open);
  const cards = (list: SettingsPart[]) => list.map((part) => <Slot key={part} node={nodes[part]} />);
  const allSet = groups.needs.length === 0 && groups.progress.length === 0;
  return (
    <>
      {groups.needs.length > 0 && (
        <Group id="settings-needs" title="Needs you" count={groups.needs.length} attention>
          {cards(groups.needs)}
        </Group>
      )}
      {groups.progress.length > 0 && (
        <Group id="settings-progress" title="In progress" count={groups.progress.length} forceOpen={holds(groups.progress)}>
          {cards(groups.progress)}
        </Group>
      )}
      {groups.ok.length > 0 && (
        <Group
          id="settings-ok"
          title="All set"
          count={groups.ok.length}
          defaultOpen={!allSet}
          forceOpen={holds(groups.ok)}
          summary={allSetSummary(groups)}
        >
          {cards(groups.ok)}
        </Group>
      )}
    </>
  );
}

/** The parts the groups show now, in the page's order. */
function placed(groups: SettingsGroups): SettingsPart[] {
  return [...groups.needs, ...groups.progress, ...groups.ok];
}

/** Puts a card's element here. When the card moves to another group, the
 * element moves with it and the card inside stays as it was. */
function Slot({ node }: { node: HTMLElement }) {
  const box = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const parent = box.current;
    if (parent === null) return;
    parent.appendChild(node);
    return () => {
      if (node.parentNode === parent) parent.removeChild(node);
    };
  }, [node]);
  return <div ref={box} />;
}
