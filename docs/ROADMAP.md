# Project "Aura Pulse" (System & Clipboard Manager) - Development Roadmap

> **Note:** This is the original historical build brief, not a live task
> list — it does not reflect current status. See [CHANGELOG.md](../CHANGELOG.md)
> for what has actually shipped.

This document outlines the required features, enhancements, and architectural requirements for the "Aura Pulse" application. The project is an existing application that requires significant **additions** and **fixes** to reach its full potential as an integrated system monitor, AI-enhanced clipboard manager, and terminal workspace.

---

## 1. Core Architecture & Stack Recommendation
To handle the requirements (system-level access, high-performance UI, fluid graphics, and cross-platform compatibility), the **Developer Agent is requested to select the most appropriate language/framework stack**, balancing performance for hardware monitoring and ease of development for the UI.

* **Constraint:** **Electron is explicitly discarded.** Please propose alternative cross-platform frameworks (e.g., Tauri, Wails) that support native system-level access and high-performance UI rendering.
* **Performance Logic:** Use high-performance backends (e.g., Rust or Go) for hardware sensors, telemetry data fetching (CPU/GPU/Watts), and AI processing to ensure the UI remains fluid.

---

## 2. Feature Additions

### A. AI-Enhanced Clipboard Manager ("The Vault")
* **Clipboard Listener:** Maintain a history of up to 5,000 clips.
* **AI Processing:**
    * **OCR:** Extract text from image clips.
    * **Vision/Analysis:** Describe images (colors, shapes, environment).
    * **Generation:** Convert images into design files (JSON/configuration) or markdown.
* **Multi-Modal Input:** Tabs for Text, Audio (transcription/description), and Image.
* **Activity Log:** Global search functionality across the 5,000-clip history.
* **Persistence:** Local database with a "Wipe All" function.

### B. System Telemetry & Monitor ("Diagnostics")
* **Fluid Visualization:** High-frequency polling (0.1s intervals).
* **Metrics Tracking:**
    * **Hardware:** CPU/GPU/VRAM load, temperature, disk usage, swap space.
    * **Power:** Real-time wattage consumption (Shader usage).
    * **Cyberpunk Graphs:** Custom animated charts/pies (high fluidity).
* **Optimizations:**
    * System optimization toggles.
    * Power profiles: Performance, Balanced, Power Saver.
    * Hardware Benchmarking Suite (specifically for LLM/AI performance estimation).

### C. UI/UX: The "Tron/Cyberpunk" Aesthetic
* **Layout:** Retractable left sidebar containing settings and the "Global Sphere."
* **The Global Sphere:** An interactive, spinning sphere of nodes/dots, connected by lines to "pinging" satellites; color schemes must reflect a "breathing" effect.
* **Dashboard Tabs:** Top-level navigation: `Diagnostics`, `Benchmarks`, `Optimization`, `The Vault`.

### D. AI Configuration Hub
* **Provider Support:** OpenAI, Anthropic, Gemini, DeepSeek, Xiaomi, Mimo, and Local (Ollama/LM Studio).
* **Custom Local Endpoints:** Ability to set Base URL, Model Name, and Port mapping for local inferencing.

---

## 3. Necessary Fixes & Refinements

* **Performance:** Ensure high-frequency telemetry polling (0.1s) does not impact main-thread UI performance.
* **Stability:** Ensure local database integrity during massive backups.
* **Integration:** Resolve discrepancies between API provider settings and actual calls.

---

## 4. Development Agent Instructions
* **Focus:** Treat this as a refactoring and feature-expansion project.
* **Priority:** Select a high-performance framework (Non-Electron) that offers tight integration with OS-level hardware sensors.
* **Naming:** Maintain the "Tron/Cyberpunk" visual style across all components.
