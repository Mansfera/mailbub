import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n";
import "./index.css";
import App from "./App";
import { SettingsWindow } from "./components/SettingsWindow";
import { IS_ANDROID } from "./lib/core";
import { blockStrayDrops } from "./lib/draftFiles";

const isSettings = window.location.hash.startsWith("#settings");
// Only the main window has the translucent sidebar material behind the page.
if ("__TAURI_INTERNALS__" in window && !isSettings && !IS_ANDROID)
  document.documentElement.dataset.shell = "tauri";
if (IS_ANDROID) document.documentElement.dataset.platform = "android";
blockStrayDrops();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>{isSettings ? <SettingsWindow /> : <App />}</React.StrictMode>,
);
