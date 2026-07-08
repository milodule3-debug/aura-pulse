import "@fontsource/orbitron/500.css";
import "@fontsource/orbitron/700.css";
import "@fontsource/rajdhani/400.css";
import "@fontsource/rajdhani/500.css";
import "@fontsource/rajdhani/600.css";
import "@fontsource-variable/jetbrains-mono";

import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/layout.css";
import "./styles/views.css";

import { App } from "./app";
import { maybeRunSelftest } from "./selftest";

// Restore saved theme before first paint to prevent flash.
const savedTheme = localStorage.getItem("ap-theme");
if (savedTheme && savedTheme !== "neon") {
  document.body.className = `theme-${savedTheme}`;
}

// The window is created with dragDropEnabled: false (native interception
// breaks HTML5 drops on webkit2gtk), so block the webview's default
// behavior of navigating to files dropped outside a drop zone.
window.addEventListener("dragover", (e) => e.preventDefault());
window.addEventListener("drop", (e) => e.preventDefault());

new App(document.getElementById("app")!);
maybeRunSelftest();
