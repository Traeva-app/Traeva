// The app's sidebar: the pet logo, one row per page, Settings and the
// switch for the desktop pet (#208) at the bottom, and the line on its right
// edge that the user drags to make it wider or narrower. Narrower than 120 px it snaps to
// the icon rail (sidebar-width.ts). Double-click the line, or Enter on it,
// switches between the rail and the full sidebar. In a narrow window the
// rail shows without the line, and the kept width comes back when the
// window is wider again.

import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from "react";
import petUrl from "./assets/traeva-pet.png";
import {
  DEFAULT,
  MAX_FULL,
  NARROW_WINDOW,
  RAIL,
  isRail,
  loadWidth,
  saveWidth,
  shownWidth,
  snapWidth,
  stepWidth,
  toggleWidth,
} from "./sidebar-width";
import "./AppShell.css";

export type Page = "find" | "organize" | "memory" | "source" | "activity" | "settings";

// The rail's labels are 11 px under 64 px icons: Source Memory is
// Sources there.
const PAGES: { page: Page; label: string; railLabel?: string; icon: ReactNode }[] = [
  { page: "find", label: "Find", icon: <SearchIcon /> },
  { page: "organize", label: "Organize", icon: <TrayIcon /> },
  { page: "memory", label: "Memory", icon: <HistoryIcon /> },
  { page: "source", label: "Source Memory", railLabel: "Sources", icon: <LinkIcon /> },
  { page: "activity", label: "Activity", icon: <ActivityIcon /> },
];

type Props = {
  page: Page;
  onSelect: (page: Page) => void;
  /** Things in Organize that wait for the user (0: none). */
  needsYou: number;
  /** The desktop pet shows (the user may hide it). */
  petShown: boolean;
  onPetShown: (shown: boolean) => void;
};

export function Sidebar({ page, onSelect, needsYou, petShown, onPetShown }: Props) {
  const [width, setWidth] = useState(loadWidth);
  const rail = isRail(width);
  const [windowWidth, setWindowWidth] = useState(() => window.innerWidth);
  useEffect(() => {
    const onResize = () => setWindowWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  const shown = shownWidth(width, windowWidth);
  const showRail = isRail(shown);
  // The line changes the kept width, so it goes while a narrow window shows
  // the rail.
  const narrowWindow = windowWidth < NARROW_WINDOW;
  // The full width to go back to from the rail.
  const lastFull = useRef(rail ? DEFAULT : width);
  const drag = useRef<{ startX: number; startWidth: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  // Kept once a drag ends, and after every other change.
  useEffect(() => {
    if (!dragging) saveWidth(width);
  }, [width, dragging]);

  // A change by key or double-click; the full width left is remembered.
  const resize = (next: number) => {
    if (!rail) lastFull.current = width;
    setWidth(next);
  };

  const startDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    if (!rail) lastFull.current = width;
    drag.current = { startX: event.clientX, startWidth: width };
    setDragging(true);
  };
  const moveDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (drag.current === null) return;
    setWidth(snapWidth(drag.current.startWidth + event.clientX - drag.current.startX));
  };
  const endDrag = () => {
    drag.current = null;
    setDragging(false);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      resize(stepWidth(width, event.key === "ArrowLeft" ? -1 : 1));
    } else if (event.key === "Enter") {
      event.preventDefault();
      resize(toggleWidth(width, lastFull.current));
    }
  };

  return (
    <>
      <nav
        id="app-sidebar"
        className={showRail ? "app-sidebar app-sidebar-rail" : "app-sidebar"}
        style={{ width: shown }}
        aria-label="Traeva features"
      >
        <div className="app-brand">
          <img src={petUrl} alt={showRail ? "Traeva" : ""} width={28} height={28} />
          {!showRail && <span>Traeva</span>}
        </div>
        {PAGES.map((item) => {
          const count = item.page === "organize" ? needsYou : 0;
          return (
            <button
              key={item.page}
              type="button"
              className="app-nav-item"
              aria-current={page === item.page ? "page" : undefined}
              onClick={() => onSelect(item.page)}
            >
              <span className="app-nav-icon">
                {item.icon}
                {showRail && count > 0 && <span className="app-nav-dot" />}
              </span>
              <span className="app-nav-label">{showRail ? (item.railLabel ?? item.label) : item.label}</span>
              {!showRail && count > 0 && (
                <span className="app-nav-count" aria-hidden="true">
                  {count}
                </span>
              )}
              {count > 0 && <span className="sr-only">, {count === 1 ? "1 thing needs you" : `${count} things need you`}</span>}
            </button>
          );
        })}
        <button
          type="button"
          className="app-nav-item app-nav-settings"
          aria-current={page === "settings" ? "page" : undefined}
          onClick={() => onSelect("settings")}
        >
          <span className="app-nav-icon">
            <GearIcon />
          </span>
          <span className="app-nav-label">Settings</span>
        </button>
        <button
          type="button"
          role="switch"
          aria-checked={petShown}
          className="app-nav-item app-pet-switch"
          title={petShown ? "Hide the pet on the desktop" : "Show the pet on the desktop"}
          onClick={() => onPetShown(!petShown)}
        >
          <span className="app-nav-icon">
            <PetIcon />
          </span>
          <span className="app-nav-label">{showRail ? "Pet" : "Desktop pet"}</span>
          {!showRail && (
            <span className="app-pet-state" aria-hidden="true">
              {petShown ? "On" : "Off"}
            </span>
          )}
        </button>
      </nav>
      {!narrowWindow && (
        <div
          className={dragging ? "app-splitter app-splitter-dragging" : "app-splitter"}
          role="separator"
          aria-orientation="vertical"
          aria-controls="app-sidebar"
          aria-label="Sidebar width"
          aria-valuemin={RAIL}
          aria-valuemax={MAX_FULL}
          aria-valuenow={width}
          aria-valuetext={rail ? "Icons only" : `${width} pixels`}
          tabIndex={0}
          title="Drag to resize. Double-click to show or hide the labels."
          onPointerDown={startDrag}
          onPointerMove={moveDrag}
          onPointerUp={endDrag}
          onPointerCancel={endDrag}
          onLostPointerCapture={endDrag}
          onDoubleClick={() => resize(toggleWidth(width, lastFull.current))}
          onKeyDown={onKeyDown}
        />
      )}
    </>
  );
}

// Line icons, 20 px, drawn with the text colour.

function Icon({ children }: { children: ReactNode }) {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.75"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

function SearchIcon() {
  return (
    <Icon>
      <circle cx="11" cy="11" r="6.5" />
      <path d="m16 16 4.5 4.5" />
    </Icon>
  );
}

function TrayIcon() {
  return (
    <Icon>
      <path d="M3.5 13.5 6 5.5h12l2.5 8" />
      <path d="M3.5 13.5V18a1.5 1.5 0 0 0 1.5 1.5h14a1.5 1.5 0 0 0 1.5-1.5v-4.5h-5l-1.5 2.5h-4L8.5 13.5z" />
    </Icon>
  );
}

function HistoryIcon() {
  return (
    <Icon>
      <path d="M4 12a8 8 0 1 0 2.4-5.7" />
      <path d="M4 4.5V8h3.5" />
      <path d="M12 8v4.5l3 2" />
    </Icon>
  );
}

function LinkIcon() {
  return (
    <Icon>
      <path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1" />
      <path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1" />
    </Icon>
  );
}

/** A list with a clock: what happened lately. */
function ActivityIcon() {
  return (
    <Icon>
      <path d="M4 6h16M4 11h6M4 16h4.5" />
      <circle cx="16.5" cy="15.5" r="4.5" />
      <path d="M16.5 13.3v2.4l1.5 1" />
    </Icon>
  );
}

function PetIcon() {
  return (
    <Icon>
      <path d="M3.5 8A1.5 1.5 0 0 1 5 6.5h4l2 2h8A1.5 1.5 0 0 1 20.5 10v7.5A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z" />
      <path d="M9.5 13.5v.5M14.5 13.5v.5" />
    </Icon>
  );
}

function GearIcon() {
  return (
    <Icon>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3.5v2.2M12 18.3v2.2M3.5 12h2.2M18.3 12h2.2M6 6l1.6 1.6M16.4 16.4 18 18M6 18l1.6-1.6M16.4 7.6 18 6" />
    </Icon>
  );
}
