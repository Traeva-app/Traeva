import { useCallback, useState } from "react";
import { ActivityPage } from "./features/activity/ActivityPage";
import type { DropOutcome } from "./features/autodrop/queue";
import { FindPage } from "./features/find/FindPage";
import { MemoryPage } from "./features/memory/MemoryPage";
import { OrganizePage } from "./features/organize/OrganizePage";
import { SettingsPage } from "./features/settings/SettingsPage";
import { SourcePage } from "./features/source/SourcePage";
import type { MainPage } from "./features/pet/api";
import { usePetLink } from "./pet/usePetLink";
import { Sidebar, type Page } from "./Sidebar";

/** The app shell: a sidebar with one row per feature, and the page beside
 * it. Find (#198) is the first page; Memory (#35), Source Memory (#27),
 * Activity (#248) and Settings (folders, model, automatic mode) load the
 * first time their row is chosen. Hidden pages stay mounted, so
 * switching keeps each page's state, and Organize handles files dropped on
 * the icon (#168) even while another page shows: a new drop, or one that
 * comes to need the user, brings Organize forward. Organize's count of
 * what waits for the user shows next to it in the sidebar; Find lists
 * those things on its home. The desktop pet (#208) shows unless the user
 * hid it; a click on it brings this window forward on Find or Organize. */
export function App() {
  const [page, setPage] = useState<Page>("find");
  const showOrganize = useCallback(() => setPage("organize"), []);
  const [memoryOpened, setMemoryOpened] = useState(false);
  const [sourceOpened, setSourceOpened] = useState(false);
  const [activityOpened, setActivityOpened] = useState(false);
  const [settingsOpened, setSettingsOpened] = useState(false);
  // The popup's "Turn on automatic mode…": Settings with that card in view,
  // bumped for each ask.
  const [revealAutomatic, setRevealAutomatic] = useState(0);
  const fromPet = useCallback((next: MainPage) => {
    if (next === "automatic") {
      setSettingsOpened(true);
      setRevealAutomatic((count) => count + 1);
      setPage("settings");
      return;
    }
    setPage(next);
  }, []);
  const { petHidden, setPetHidden } = usePetLink(fromPet);
  const [needsYou, setNeedsYou] = useState(0);
  const [dropOutcomes, setDropOutcomes] = useState<Record<string, DropOutcome>>({});
  // The record Find's Versions asked Memory to show, until Memory has
  // handled it (cleared then, so the same record can be asked for again).
  const [openChainId, setOpenChainId] = useState<string | null>(null);
  // Find's Review on an action that needs a decision: Organize brings that
  // card into view, then clears this.
  const [showDecisions, setShowDecisions] = useState(false);
  const decisionsShown = useCallback(() => setShowDecisions(false), []);
  // Activity's Undo… on an action: Organize starts its usual Undo (a
  // preview first), then clears this.
  const [undoActionId, setUndoActionId] = useState<string | null>(null);
  const undoStarted = useCallback(() => setUndoActionId(null), []);
  // Another page the user picks opens at its top (the window scrolls; the
  // sidebar stays). Not when a dropped file brings Organize forward
  // (showOrganize): its queue card scrolls the file into view.
  const select = (next: Page) => {
    if (next === "memory") setMemoryOpened(true);
    if (next === "source") setSourceOpened(true);
    if (next === "activity") setActivityOpened(true);
    if (next === "settings") setSettingsOpened(true);
    if (next !== page) window.scrollTo(0, 0);
    setPage(next);
  };
  const goOrganize = (to?: "decisions") => {
    setShowDecisions(to === "decisions");
    select("organize");
  };
  const undoInOrganize = (actionId: string) => {
    setUndoActionId(actionId);
    goOrganize();
  };
  const showVersions = (chainId: string) => {
    setOpenChainId(chainId);
    select("memory");
  };
  const chainOpened = useCallback(() => setOpenChainId(null), []);
  return (
    <div className="app-shell">
      <Sidebar
        page={page}
        onSelect={select}
        needsYou={needsYou}
        petShown={!petHidden}
        onPetShown={(shown) => setPetHidden(!shown)}
      />
      <div className="app-content">
        <div hidden={page !== "find"}>
          <FindPage
            active={page === "find"}
            onOrganize={goOrganize}
            onVersions={showVersions}
            dropOutcomes={dropOutcomes}
          />
        </div>
        <div hidden={page !== "organize"}>
          <OrganizePage
            active={page === "organize"}
            onShowDrop={showOrganize}
            onNeedsYou={setNeedsYou}
            onDropOutcomes={setDropOutcomes}
            showDecisions={showDecisions}
            onDecisionsShown={decisionsShown}
            undoActionId={undoActionId}
            onUndoStarted={undoStarted}
          />
        </div>
        {memoryOpened && (
          <div hidden={page !== "memory"}>
            <MemoryPage active={page === "memory"} openChainId={openChainId} onChainOpened={chainOpened} />
          </div>
        )}
        {sourceOpened && (
          <div hidden={page !== "source"}>
            <SourcePage active={page === "source"} />
          </div>
        )}
        {activityOpened && (
          <div hidden={page !== "activity"}>
            <ActivityPage
              active={page === "activity"}
              onOrganize={goOrganize}
              onUndo={undoInOrganize}
              dropOutcomes={dropOutcomes}
            />
          </div>
        )}
        {settingsOpened && (
          <div hidden={page !== "settings"}>
            <SettingsPage active={page === "settings"} revealAutomatic={revealAutomatic} />
          </div>
        )}
      </div>
    </div>
  );
}
