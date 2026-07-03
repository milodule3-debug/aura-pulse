#!/usr/bin/env node
// Launcher for the prebuilt Aura Pulse binary shipped inside this package.

import { spawn } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

if (process.platform !== "linux" || process.arch !== "x64") {
  console.error(
    `aura-pulse: prebuilt binary is Linux x64 only (you are on ${process.platform}/${process.arch}).\n` +
      "Build from source instead: https://github.com/milodule3-debug/aura-pulse",
  );
  process.exit(1);
}

const bin = path.join(here, "..", "native", "aura-pulse");
if (!existsSync(bin)) {
  console.error("aura-pulse: bundled binary missing — corrupted install? Try reinstalling the package.");
  process.exit(1);
}

const child = spawn(bin, process.argv.slice(2), { stdio: "inherit", detached: false });
child.on("error", (err) => {
  console.error(`aura-pulse: failed to launch: ${err.message}`);
  console.error("Runtime deps required: webkit2gtk 4.1 / gtk3 (Ubuntu: sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0)");
  process.exit(1);
});
child.on("exit", (code, signal) => process.exit(signal ? 1 : (code ?? 0)));
