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

new App(document.getElementById("app")!);
maybeRunSelftest();
