import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { App } from "./App";
import { PetApp } from "./pet/PetApp";
import { QuickSearch } from "./quick/QuickSearch";
import { DropPopup } from "./popup/DropPopup";
import "./styles.css";

/** Which of Traeva's windows this page is: the desktop pet's (#208) shows
 * only the pet, the quick search window only the search, the pet's popup
 * ("ask", #37) one dropped file that waits for the user. */
function windowLabel(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    // Not inside Tauri (a plain browser): the app.
    return "main";
  }
}

const label = windowLabel();
// Before the first paint, so the pet's window never shows a background.
if (label === "pet") document.documentElement.classList.add("pet-window");
if (label === "quick") document.documentElement.classList.add("quick-window");
if (label === "ask") document.documentElement.classList.add("ask-window");

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    {label === "pet" ? <PetApp /> : label === "quick" ? <QuickSearch /> : label === "ask" ? <DropPopup /> : <App />}
  </StrictMode>,
);
