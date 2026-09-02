# callRoot

**Root** is a fast, local desktop document indexing and management application built with [Tauri v2](https://v2.tauri.app/), [React](https://react.dev/), [TypeScript](https://www.typescriptlang.org/), and [Vite](https://vitejs.dev/).

## Features

- **Folder Monitoring**: Track multiple local directories for automated document discovery.
- **Real-time Filesystem Watching**: Uses Rust's `notify` and `walkdir` to detect file additions, modifications, and deletions.
- **Embedded Local Database**: Fast SQLite database (`rusqlite` in WAL mode) storing document metadata locally.
- **Supported File Formats**: Supports indexing for PDF, DOCX, Markdown, Text, CSV, Excel, PowerPoint, JSON, YAML, HTML, and more.
- **Instant Search & Sorting**: Search documents by name and sort by size, modification date, or file name.
- **Direct System Open**: Launch indexed documents in your OS default applications directly from the UI.

## Tech Stack

- **Backend**: Rust, Tauri v2, SQLite (`rusqlite`), Tokio, Notify
- **Frontend**: React 18, TypeScript, Vite, Lucide Icons, Vanilla CSS

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/) and `cargo`

### Installation

```bash
# Install frontend dependencies
npm install

# Run application in development mode
npm run dev
# or with Tauri desktop window:
npm run tauri dev
```

### Build

```bash
npm run tauri build
```
