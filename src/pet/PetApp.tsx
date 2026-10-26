// The desktop pet (#208): a 64 px folder, the size of a desktop folder
// icon, in its own small transparent window. Its face follows the drop
// queue (#168), and a small bubble counts the files that wait for the user
// (#248). A newly dropped file makes it hop, three dots show while the
// core works, and "Needs you" shows for a moment when a file comes to wait
// (#37, 10/14). A click opens the quick search window (or Traeva, when
// something waits for the user), a drag moves the window. Files
// dropped on it reach the same queue in Rust: this page gets no drag
// events and no dropped paths, and what it reads of the queue
// (get_pet_state) holds no paths, names or reasons.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  NO_BUBBLE,
  bubbleText,
  clickTarget,
  faceLayers,
  firstFaces,
  moodAt,
  needsUserCount,
  nextBubble,
  nextFaces,
  petLabel,
  pollMs,
  readQueue,
  settleFaces,
  speechAt,
  startState,
} from "./mood";
import type { Bubble, Faces, Mood, PetState, Speech } from "./mood";
import { getPetSignal, getPetState, showMainWindow, showQuickSearch } from "../features/pet/api";
import { showDropPopup, showMovedNote } from "../popup/api";
import { NO_NOTES, dropsCaught, readNotes } from "./note";
import type { PetStateView } from "../features/pet/api";
import idle from "./assets/pet-idle.png";
import thinking from "./assets/pet-thinking.png";
import question from "./assets/pet-question.png";
import wink from "./assets/pet-wink.png";
import sad from "./assets/pet-sad.png";
import "./pet.css";

const FACES: Record<Mood, string> = { idle, thinking, question, wink, sad };

// A press that moves further than this drags the window instead of clicking.
const DRAG_PX = 4;

// The older faces have gone out by then (pet.css: 260 ms at most).
const FACE_LEAVE_MS = 400;

// The hop when a dropped file is caught: up, a small squash, settled. Only
// when the system allows motion.
const HOP: Keyframe[] = [
  { transform: "translateY(0) scale(1, 1)" },
  { transform: "translateY(-7px) scale(0.97, 1.04)", offset: 0.35 },
  { transform: "translateY(0) scale(1.05, 0.94)", offset: 0.65 },
  { transform: "translateY(0) scale(1, 1)" },
];

function moving(): boolean {
  return !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

// The quick signal (a file over the pet, drops caught) is read this often,
// so the pet reacts at once; the queue itself is read as pollMs says, and
// at once when a drop was caught.
const SIGNAL_POLL_MS = 300;

// What the bubble says now: a file over the pet comes first.
type ShownSpeech = Speech | "drop";

interface View {
  queue: PetStateView | null;
  pet: PetState;
}

export function PetApp() {
  const [view, setView] = useState<View>(() => ({ queue: null, pet: startState(Date.now()) }));

  // Read the queue, then again after pollMs; a failed read keeps the last
  // face (idle if none was read yet). The loop keeps the latest view itself
  // and hands each new one to React.
  const latest = useRef(view);
  // Reads the queue now (the signal calls it when a drop was caught); a call
  // during a read reads once more right after it.
  const readNow = useRef<() => void>(() => undefined);
  // Files dropped alone on the pet, for the "Moved to" note (note.ts).
  const notes = useRef(NO_NOTES);
  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    let reading = false;
    let again = false;
    const read = () => {
      if (reading) {
        again = true;
        return;
      }
      reading = true;
      window.clearTimeout(timer);
      getPetState()
        .then((queue) => {
          if (stopped) return;
          latest.current = { queue, pet: readQueue(latest.current.pet, queue, Date.now()) };
          setView(latest.current);
          const read = readNotes(notes.current, queue);
          notes.current = read.state;
          if (read.note !== null) showMovedNote(read.note).catch(() => undefined);
        })
        .catch(() => undefined)
        .finally(() => {
          reading = false;
          if (stopped) return;
          if (again) {
            again = false;
            read();
            return;
          }
          timer = window.setTimeout(read, pollMs(latest.current.queue, latest.current.pet, Date.now()));
        });
    };
    readNow.current = read;
    read();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, []);

  // The quick signal: "Drop it here" while a file is over the pet, and the
  // queue read at once when a drop was caught (the hop comes with it).
  const [dragOver, setDragOver] = useState(false);
  useEffect(() => {
    let stopped = false;
    let timer: number | undefined;
    let drops: number | null = null;
    const look = () => {
      const next = () => {
        if (!stopped) timer = window.setTimeout(look, SIGNAL_POLL_MS);
      };
      if (document.hidden) {
        next();
        return;
      }
      getPetSignal()
        .then((signal) => {
          if (stopped) return;
          setDragOver(signal.drag_over);
          if (drops !== null && signal.drops !== drops) {
            notes.current = dropsCaught(notes.current, signal.drops - drops);
            readNow.current();
          }
          drops = signal.drops;
        })
        .catch(() => undefined)
        .finally(next);
    };
    look();
    return () => {
      stopped = true;
      window.clearTimeout(timer);
    };
  }, []);

  // Render again when a wink or sad face, or "Needs you", ends.
  const [, setTick] = useState(0);
  const flash = view.pet.flash;
  useEffect(() => {
    if (flash === null) return;
    const left = flash.until - Date.now();
    if (left <= 0) return;
    const timer = window.setTimeout(() => setTick((tick) => tick + 1), left);
    return () => window.clearTimeout(timer);
  }, [flash]);
  const asked = view.pet.asked;
  useEffect(() => {
    if (asked === null) return;
    const left = asked - Date.now();
    if (left <= 0) return;
    const timer = window.setTimeout(() => setTick((tick) => tick + 1), left);
    return () => window.clearTimeout(timer);
  }, [asked]);

  // A newly dropped file: the folder hops once, at the read that found it.
  const stage = useRef<HTMLDivElement>(null);
  const catches = view.pet.catches;
  useEffect(() => {
    if (catches === 0 || !moving()) return;
    stage.current?.animate(HOP, { duration: 480, easing: "ease-out" });
  }, [catches]);

  const mood = moodAt(view.pet, view.queue, Date.now());
  const speech: ShownSpeech = dragOver ? "drop" : speechAt(view.pet, view.queue, Date.now());
  const count = needsUserCount(view.queue);
  const label = petLabel(mood, count);

  // A new mood or count updates the faces and the bubble before this
  // render is shown, so the old face is still there when the new one comes
  // in (no frame without a face).
  const [faces, setFaces] = useState<Faces>(() => firstFaces(mood));
  const shownFaces = nextFaces(faces, mood);
  if (shownFaces !== faces) setFaces(shownFaces);
  const [bubble, setBubble] = useState<Bubble>(NO_BUBBLE);
  const shownBubble = nextBubble(bubble, count);
  if (shownBubble !== bubble) setBubble(shownBubble);

  // Then the older faces go.
  useEffect(() => {
    if (faces.layers.length === 1) return;
    const timer = window.setTimeout(() => setFaces(settleFaces), FACE_LEAVE_MS);
    return () => window.clearTimeout(timer);
  }, [faces]);

  // Something waits for the user: Organize in the main window. Otherwise the
  // quick search window, or Find in the main window if it cannot show.
  const open = () => {
    const target = clickTarget(view.queue);
    if (target === "popup") {
      showDropPopup().catch(() => showMainWindow("organize").catch(() => undefined));
      return;
    }
    if (target === "organize") {
      showMainWindow("organize").catch(() => undefined);
      return;
    }
    showQuickSearch().catch(() => showMainWindow("find").catch(() => undefined));
  };

  // Click or drag: a press that moves hands the window to the system
  // (a drag region would swallow the click).
  const press = useRef<{ x: number; y: number } | null>(null);
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    press.current = { x: event.screenX, y: event.screenY };
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const start = press.current;
    if (start === null) return;
    if (Math.hypot(event.screenX - start.x, event.screenY - start.y) <= DRAG_PX) return;
    press.current = null;
    getCurrentWindow()
      .startDragging()
      .catch(() => undefined);
  };
  const onPointerUp = () => {
    if (press.current === null) return;
    press.current = null;
    open();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Enter" && event.key !== " ") return;
    event.preventDefault();
    if (!event.repeat) open();
  };

  // "Needs you" takes the count's place for a moment; the dots show only
  // when nothing waits for the user (speechAt); "Drop it here" covers both.
  const bubbleShown = shownBubble.count > 0 && speech !== "needs" && speech !== "drop";
  const petClass = ["pet", bubbleShown && "pet-with-bubble", speech && `pet-with-${speech}`]
    .filter(Boolean)
    .join(" ");

  return (
    <div
      className={petClass}
      role="button"
      tabIndex={0}
      aria-label={label}
      title={label}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={() => {
        press.current = null;
      }}
      onKeyDown={onKeyDown}
    >
      <div className="pet-stage" ref={stage}>
        {faceLayers(shownFaces).map((layer) => (
          <Face key={layer.key} mood={layer.mood} leaving={layer.leaving} />
        ))}
      </div>
      {speech === "working" && (
        // Says the same as the label ("working"); takes no pointer events.
        <span className="pet-speech pet-speech-dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      )}
      {speech === "needs" && (
        <span className="pet-speech pet-speech-needs" aria-hidden="true">
          Needs you
        </span>
      )}
      {speech === "drop" && (
        <span className="pet-speech pet-speech-drop" aria-hidden="true">
          Drop it here
        </span>
      )}
      {bubbleShown && (
        // Says the same as the label; takes no pointer events (pet.css).
        <span className="pet-bubble" aria-hidden="true">
          {/* A new key per bump replays it. */}
          <span key={shownBubble.bumps} className={shownBubble.bumps > 0 ? "pet-bubble-dot pet-bump" : "pet-bubble-dot"}>
            {bubbleText(shownBubble.count)}
          </span>
        </span>
      )}
    </div>
  );
}

// One face. It comes in with pet.css's pet-face-in. When a newer face comes,
// this one plays the same motion backwards from wherever it is: a face that
// was still coming in goes back out without a jump, and a face that was
// fully in fades out while the new one covers it. With reduced motion there
// is no motion, and pet.css hides an old face at once. A WebView without
// getAnimations (before macOS 10.15.4) leaves it in place until the faces
// settle.
function Face({ mood, leaving }: { mood: Mood; leaving: boolean }) {
  const layer = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    if (!leaving || layer.current === null || typeof layer.current.getAnimations !== "function") return;
    for (const motion of layer.current.getAnimations()) {
      // Not begun yet, so never seen: it stays at its start, unseen.
      // (Played backwards from its start, it would rewind to the end.)
      if (typeof motion.currentTime === "number" && motion.currentTime > 0) motion.reverse();
      else motion.pause();
    }
  }, [leaving]);
  return (
    <div ref={layer} className={leaving ? "pet-layer pet-leave" : "pet-layer"}>
      <img
        className={`pet-face pet-${mood}`}
        src={FACES[mood]}
        alt=""
        width={64}
        height={64}
        draggable={false}
      />
    </div>
  );
}
