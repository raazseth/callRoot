# 📁 Root — Local Document Indexer

> **Find what you need in milliseconds, without the resource bloat.**  
> A fast, lightweight, and 100% local desktop document organizer built with **Tauri v2**, **Rust**, and **React**.

---

## 💡 The "Why": The Problem Root Solves

We have all been there: you downloaded a contract, drafted a project specification, or saved a research paper somewhere on your drive weeks ago. Now you need it urgently, but:

- **Built-in OS searches** can be sluggish, unpredictable, or index things you don’t care about.
- **Heavy desktop tools** built on bulky frameworks often consume 500MB–1GB+ of RAM just idling in the background.
- **Cloud search services** compromise your privacy by requiring your local documents to be uploaded or synced.

**Root** was built to solve this simply and elegantly:
1. **You choose the folders that matter** (e.g., your Projects, Documents, or Downloads directory).
2. **Root keeps an ultra-fast local index** of all your documents using an embedded SQLite database.
3. **Rust watches your files in real-time**: when you add, edit, or delete a document, Root reflects the change instantly.
4. **Instant search & launch**: Search as you type, sort by date or size, and open the document directly in your default system app with one click.

**Zero cloud sync. Zero telemetry. Minimal memory footprint.**

---

## ✨ Features

- ⚡ **Lightning Fast & Lightweight**: Powered by native Rust; uses only a fraction of the memory and CPU of traditional Electron-based indexers.
- 📂 **Multi-Directory Monitoring**: Watch multiple custom folders simultaneously.
- 🔄 **Real-Time Filesystem Watching**: Instant detection of file creation, edits, and removals via filesystem events.
- 🔍 **Instant Case-Insensitive Search**: Filter hundreds or thousands of documents with zero lag as you type.
- 📊 **Smart Sorting**: Sort your indexed documents by modification date, file size, name, or extension.
- 🚀 **Direct System Open**: Launch files directly in your operating system's native viewer (Adobe Acrobat, Word, VS Code, etc.).
- 🔒 **100% Local & Privacy-Focused**: Your files and metadata never leave your computer. Everything resides in an embedded SQLite database on your machine.

---

## 🛠️ Supported Document Formats

Root automatically identifies, indexes, and categorizes common document formats while filtering out hidden files, system directories, and build artifacts:

| Category | File Extensions |
| :--- | :--- |
| **Documents** | `.pdf`, `.docx`, `.doc`, `.odt`, `.rtf`, `.epub` |
| **Text & Notes** | `.txt`, `.md`, `.markdown` |
| **Data & Spreadsheets** | `.csv`, `.xlsx`, `.xls` |
| **Presentations** | `.pptx`, `.ppt` |
| **Code & Config** | `.json`, `.yaml`, `.yml`, `.xml`, `.html`, `.htm`, `.log` |

> Root intentionally excludes common heavy directories like `node_modules`, `.git`, `target`, and `dist` to keep indexing fast and clean.

---

## 🏗️ Tech Stack & Architecture

Root leverages a modern hybrid desktop architecture: a high-performance **Rust** core handling OS-level interactions, paired with a snappy **React** user interface.

```text
┌────────────────────────────────────────────────────────┐
│                   React + TypeScript                   │
│             (Vite + Lucide Icons + CSS)                │
└──────────────────────────┬─────────────────────────────┘
                           │ Tauri IPC (Commands & Events)
┌──────────────────────────▼─────────────────────────────┐
│                    Tauri v2 (Rust)                     │
│  ┌───────────────────────┬───────────────────────────┐ │
│  │   Notify & Walkdir    │     SQLite (Rusqlite)     │ │
│  │ (FS Watching & Scan)  │     (WAL Mode Storage)    │ │
│  └───────────────────────┴───────────────────────────┘ │
└────────────────────────────────────────────────────────┘
```

### Backend (Rust / Tauri v2)
- **[Tauri v2](https://v2.tauri.app/)**: Next-generation desktop application framework utilizing OS webviews for minimal binary size and memory usage.
- **[Rusqlite](https://github.com/rusqlite/rusqlite)**: Embedded SQLite database with **WAL (Write-Ahead Logging)** mode for concurrent, atomic reads and writes.
- **[Notify](https://github.com/notify-rs/notify)**: Cross-platform filesystem event monitoring for instantaneous updates.
- **[WalkDir](https://github.com/BurntSushi/walkdir)**: Fast, recursive directory traversal for initial background scans.
- **[Tokio](https://tokio.rs/)**: Asynchronous runtime managing background indexing without freezing the UI.

### Frontend (React / TypeScript)
- **[React 18](https://react.dev/)**: Component-driven UI.
- **[TypeScript](https://www.typescriptlang.org/)**: Full type safety across Tauri IPC commands and data models.
- **[Vite](https://vitejs.dev/)**: Fast frontend development and optimized production bundling.
- **[Lucide React](https://lucide.dev/)**: Clean, modern iconography.

---

## 🚀 Getting Started

### Prerequisites

Ensure you have the following installed on your machine:
- **Node.js** (v18 or newer) & `npm`
- **Rust** & `cargo` (via [rustup.rs](https://rustup.rs/))
- Platform-specific Tauri build tools (see [Tauri Prerequisites](https://v2.tauri.app/start/prerequisites/))

### Installation

1. **Clone the repository**:
   ```bash
   git clone https://github.com/raazseth/callRoot.git
   cd callRoot
   ```

2. **Install frontend dependencies**:
   ```bash
   npm install
   ```

### Running in Development

Start the development server with live reload:

```bash
# Run desktop window + dev server
npm run tauri dev
```

Alternatively, to test the UI in your web browser:
```bash
npm run dev
```

### Production Build

To compile a production-ready desktop installer / executable:

```bash
npm run tauri build
```
The compiled installer will be available under `src-tauri/target/release/bundle/`.

---

## 🗺️ Roadmap & Future Enhancements

- [ ] **Full-Text In-Document Search**: Extracting text content from PDFs and text files for deep keyword matching.
- [ ] **Tagging & Collections**: Create custom tags to group documents across different physical folders.
- [ ] **Local AI / Semantic Search**: Optional offline embeddings (via ONNX or local models) to find files by concept ("Find my tax receipt from last June").
- [ ] **Global Shortcut**: Hotkey to bring Root up instantly from the system tray.

---

## 📄 License

This project is licensed under the [MIT License](LICENSE) — feel free to modify and use it for your own workflows!
