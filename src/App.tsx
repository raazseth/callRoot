import React, { useState, useEffect, useRef, useMemo } from 'react';
import { Sidebar } from './components/Sidebar';
import { SearchBar } from './components/SearchBar';
import { EmptyState } from './components/EmptyState';
import { DocumentList } from './components/DocumentList';
import {
  getFolders,
  addFolder,
  removeFolder,
  rescanFolder,
  getDocuments,
  openDocument,
  removeDocumentFromIndex,
  onScanProgress,
  onIndexUpdate,
} from './api/tauri';
import { WatchedFolder, DocumentRecord, ScanProgress } from './types';

export const App: React.FC = () => {
  const [folders, setFolders] = useState<WatchedFolder[]>([]);
  const [activeFolderId, setActiveFolderId] = useState<number | null>(null);
  const [documents, setDocuments] = useState<DocumentRecord[]>([]);
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [error, setError] = useState<string | null>(null);
  const [scanProgress, setScanProgress] = useState<ScanProgress>({
    processed: 0,
    total: 0,
    current_file: '',
    is_scanning: false,
  });

  // Ref so event callbacks always read the latest activeFolderId
  // without needing to re-subscribe when the folder changes.
  const activeFolderIdRef = useRef<number | null>(activeFolderId);
  useEffect(() => {
    activeFolderIdRef.current = activeFolderId;
  }, [activeFolderId]);

  // Subscribe to backend events once on mount.
  useEffect(() => {
    loadFoldersAndDocuments();

    let unlistenProgress: (() => void) | null = null;
    let unlistenIndex: (() => void) | null = null;

    // scan-progress: update progress bar
    onScanProgress((progress) => {
      setScanProgress(progress);
    }).then((fn) => {
      unlistenProgress = fn;
    });

    // index-update: IndexWorker finished a mutation batch → reload documents
    onIndexUpdate(() => {
      loadDocuments(activeFolderIdRef.current);
    }).then((fn) => {
      unlistenIndex = fn;
    });

    return () => {
      if (unlistenProgress) unlistenProgress();
      if (unlistenIndex) unlistenIndex();
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const loadFoldersAndDocuments = async () => {
    try {
      const folderList = await getFolders();
      setFolders(folderList);
      await loadDocuments(activeFolderIdRef.current);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Failed to load initial data:', msg);
      setError(`Failed to load data: ${msg}`);
    }
  };

  const loadDocuments = async (folderId?: number | null) => {
    try {
      const docs = await getDocuments(folderId ?? null);
      setDocuments(docs);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Failed to load documents:', msg);
      setError(`Failed to load documents: ${msg}`);
    }
  };

  const handleAddFolder = async () => {
    try {
      setError(null);
      const newFolder = await addFolder();
      if (newFolder) {
        await loadFoldersAndDocuments();
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Error adding watched folder:', msg);
      setError(`Failed to add folder: ${msg}`);
    }
  };

  const handleRemoveFolder = async (folderId: number) => {
    try {
      setError(null);
      await removeFolder(folderId);
      if (activeFolderId === folderId) {
        setActiveFolderId(null);
        activeFolderIdRef.current = null;
      }
      await loadFoldersAndDocuments();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Error removing folder:', msg);
      setError(`Failed to remove folder: ${msg}`);
    }
  };

  const handleRescanFolder = async (folderId: number) => {
    try {
      setError(null);
      setScanProgress({ processed: 0, total: 0, current_file: 'Initializing scan…', is_scanning: true });
      await rescanFolder(folderId);
      // Documents will reload via index-update event when reconciliation completes
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Error rescanning folder:', msg);
      setError(`Failed to rescan folder: ${msg}`);
      setScanProgress((prev) => ({ ...prev, is_scanning: false }));
    }
  };

  const handleOpenDoc = async (path: string) => {
    try {
      setError(null);
      await openDocument(path);
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Error opening document:', msg);
      setError(`Failed to open file: ${msg}`);
    }
  };

  const handleRemoveDoc = async (docId: number) => {
    try {
      setError(null);
      await removeDocumentFromIndex(docId);
      setDocuments((prev) => prev.filter((d) => d.id !== docId));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      console.error('Error removing document from index:', msg);
      setError(`Failed to remove document: ${msg}`);
    }
  };

  // Client-side search filter (fast, no additional IPC round-trip)
  const filteredDocuments = useMemo(() => {
    if (!searchQuery.trim()) return documents;
    const q = searchQuery.toLowerCase().trim();
    return documents.filter((doc) => doc.filename.toLowerCase().includes(q));
  }, [documents, searchQuery]);

  return (
    <div className="app-container">
      <Sidebar
        folders={folders}
        activeFolderId={activeFolderId}
        onSelectFolder={(id) => {
          setActiveFolderId(id);
          activeFolderIdRef.current = id;
          loadDocuments(id);
        }}
        onAddFolder={handleAddFolder}
        onRescanFolder={handleRescanFolder}
        onRemoveFolder={handleRemoveFolder}
        totalDocumentsCount={documents.length}
      />

      <main className="main-content">
        <SearchBar
          query={searchQuery}
          onQueryChange={setSearchQuery}
          resultCount={filteredDocuments.length}
        />

        {error && (
          <div
            className="error-banner"
            role="alert"
            style={{
              background: 'rgba(255,60,60,0.12)',
              border: '1px solid rgba(255,60,60,0.4)',
              borderRadius: '8px',
              padding: '10px 16px',
              margin: '8px 16px',
              fontSize: '13px',
              color: '#ff6b6b',
              display: 'flex',
              justifyContent: 'space-between',
              alignItems: 'center',
            }}
          >
            <span>{error}</span>
            <button
              onClick={() => setError(null)}
              style={{ background: 'none', border: 'none', color: '#ff6b6b', cursor: 'pointer', fontSize: '16px' }}
              aria-label="Dismiss error"
            >
              ×
            </button>
          </div>
        )}

        {folders.length === 0 ? (
          <EmptyState onAddFolder={handleAddFolder} />
        ) : (
          <DocumentList
            documents={filteredDocuments}
            onOpenDoc={handleOpenDoc}
            onRemoveDoc={handleRemoveDoc}
          />
        )}

        <footer className="status-bar">
          <div className="status-indicator">
            <span className={`status-dot ${scanProgress.is_scanning ? 'scanning' : ''}`}></span>
            <span>
              {scanProgress.is_scanning
                ? `Indexing… ${scanProgress.processed} files processed`
                : folders.length === 0
                ? 'No monitored folders'
                : `Ready • ${documents.length} active documents`}
            </span>
          </div>
          {scanProgress.is_scanning && (
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ fontSize: '11px', maxWidth: '200px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {scanProgress.current_file}
              </span>
              <div className="progress-bar-container">
                {/* Indeterminate progress — no fake total */}
                <div
                  className="progress-bar-fill progress-bar-indeterminate"
                  style={{ width: '40%' }}
                />
              </div>
            </div>
          )}
        </footer>
      </main>
    </div>
  );
};

export default App;