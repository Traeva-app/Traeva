// The main window's side of the desktop pet (#208). The pet starts hidden:
// the main window shows it unless the user hid it, and when the window gets
// the focus (a click on the pet shows and focuses it), it opens the page the
// pet asked for. The main window may not listen to Tauri events, so the
// window's own focus event is the signal. After the pet shows again, the
// focus goes back where it was (focusBack.ts).

import { useCallback, useEffect, useState } from "react";
import { MAIN_FOCUS_BACK, PET_HIDDEN_KEY, setPetVisible, takePageRequest } from "../features/pet/api";
import type { MainPage } from "../features/pet/api";
import { focusBackTarget } from "./focusBack";

function readHidden(): boolean {
  try {
    return localStorage.getItem(PET_HIDDEN_KEY) === "true";
  } catch {
    return false;
  }
}

/** Shows or hides the pet as the user chose, and calls `onPage` with the
 * page the pet asked for. `setPetHidden` remembers a new choice. */
export function usePetLink(onPage: (page: MainPage) => void): {
  petHidden: boolean;
  setPetHidden: (hidden: boolean) => void;
} {
  const [petHidden, setHidden] = useState(readHidden);

  useEffect(() => {
    // Where the focus is as the switch is used: a text field (a click on a
    // button does not move it in WebKit), or the switch.
    const before = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    setPetVisible(!petHidden)
      .then(() => {
        if (petHidden) return;
        // Showing the pet took the focus and gave it back to this window
        // without a focus event here (#259). Find puts its search box back;
        // on any other page the focus goes back where it was.
        window.dispatchEvent(new Event(MAIN_FOCUS_BACK));
        focusBackTarget(before, document.activeElement, document.body)?.focus({ preventScroll: true });
      })
      .catch(() => undefined);
  }, [petHidden]);

  useEffect(() => {
    const take = () => {
      takePageRequest()
        .then((page) => {
          if (page !== null) onPage(page);
        })
        .catch(() => undefined);
    };
    take();
    window.addEventListener("focus", take);
    return () => window.removeEventListener("focus", take);
  }, [onPage]);

  const setPetHidden = useCallback((hidden: boolean) => {
    try {
      if (hidden) localStorage.setItem(PET_HIDDEN_KEY, "true");
      else localStorage.removeItem(PET_HIDDEN_KEY);
    } catch {
      // Not kept: the choice lasts until Traeva quits.
    }
    setHidden(hidden);
  }, []);

  return { petHidden, setPetHidden };
}
