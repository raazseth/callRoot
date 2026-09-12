import { invoke } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import { WatchedFolder, DocumentRecord, ScanProgress } from '../types';

export const isTauriAvailable = (): boolean => {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
};

// Folder commands
export async function addFolder(path?: string): Promise<WatchedFolder | null> {
  if (!isTauriAvailable()) {
    console.warn('Tauri API not available (web mode)');
    const selectedPath = path || 'C:\\Users\\Mock\\Documents';
    return {
      id: Date.now(),
      path: selectedPath,
      created_at: Math.floor(Date.now() / 1000),
      updated_at: Math.floor(Date.now() / 1000),
    };
  }

  let folderPath = path;
  if (!folderPath) {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: 'Select Folder to Watch in Root',
    });
    if (!selected || Array.isArray(selected)) return null;
    folderPath = selected;
  }

  return await invoke<WatchedFolder>('cmd_add_folder', { path: folderPath });
}

export async function getFolders(): Promise<WatchedFolder[]> {
  if (!isTauriAvailable()) return [];
  return await invoke<WatchedFolder[]>('cmd_get_folders');
}

export async function removeFolder(folderId: number): Promise<boolean> {
  if (!isTauriAvailable()) return true;
  return await invoke<boolean>('cmd_remove_folder', { folderId });
}

export async function rescanFolder(folderId: number): Promise<boolean> {
  if (!isTauriAvailable()) return true;
  return await invoke<boolean>('cmd_rescan_folder', { folderId });
}

// Document commands
export async function getDocuments(folderId?: number | null): Promise<DocumentRecord[]> {
  if (!isTauriAvailable()) return [];
  return await invoke<DocumentRecord[]>('cmd_get_documents', { folderId: folderId ?? null });
}

export async function searchDocuments(query: string, folderId?: number | null): Promise<DocumentRecord[]> {
  if (!isTauriAvailable()) return [];
  return await invoke<DocumentRecord[]>('cmd_search_documents', { query, folderId: folderId ?? null });
}

export async function openDocument(path: string): Promise<boolean> {
  if (!isTauriAvailable()) {
    console.log('Simulating opening document:', path);
    return true;
  }
  return await invoke<boolean>('cmd_open_document', { path });
}

export async function removeDocumentFromIndex(docId: number): Promise<boolean> {
  if (!isTauriAvailable()) return true;
  return await invoke<boolean>('cmd_remove_document', { docId });
}

// Event Subscriptions
export async function onScanProgress(callback: (progress: ScanProgress) => void): Promise<UnlistenFn> {
  if (!isTauriAvailable()) {
    return () => {};
  }
  return await listen<ScanProgress>('scan-progress', (event) => {
    callback(event.payload);
  });
}

/// Fired by IndexWorker after each batch of mutations (upsert, mark-missing, reconcile).
/// Frontend listens to this event to reload the document list reactively.
export async function onIndexUpdate(callback: () => void): Promise<UnlistenFn> {
  if (!isTauriAvailable()) {
    return () => {};
  }
  return await listen('index-update', () => {
    callback();
  });
}
