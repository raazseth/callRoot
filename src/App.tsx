import React, { useState, useEffect, useMemo } from 'react';
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
  searchDocuments,
  openDocument,
  removeDocumentFromIndex,
  onScanProgress,
  isTauriAvailable,
} from './api/tauri';
import { WatchedFolder, DocumentRecord, ScanProgress } from './types';

export const App: React.FC = () => {
  const [folders, setFolders] = useState<WatchedFolder[]>([]);
  const [activeFolderId, setActiveFolderId] = useState<number | null>(null);
  const [documents, setDocuments] = useState<DocumentRecord[]>([]);
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [scanProgress, setScanProgress] = useState<ScanProgress>({
    processed: 0,
    total: 0,
    current_file: '',
    is_scanning: false,
  });

  // Load initial data
  useEffect(() => {
    loadFoldersAndDocuments();

    let unlisten: (() => void) | null = null;
    onScanProgress((progress) => {
      setScanProgress(progress);
      if (!progress.is_scanning) {
        // Refresh documents when scanning finishes
        loadDocuments(activeFolderId);
      }
    }).then((unlistenFn) => {
      unlisten = unlistenFn;
    });

    return () => {
      if (unlisten) unlisten();
    };
  }, []);

  const loadFoldersAndDocuments = async () => {
    try {
      const folderList = await getFolders();
      setFolders(folderList);

      if (folderList.length > 0) {
        await loadDocuments(activeFolderId);
      } else {
        setDocuments([]);
      }
    } catch (err) {
      console.error('Failed to load initial data:', err);
    }
  };

  const loadDocuments = async (folderId?: number | null) => {
    try {
      const docs = await getDocuments(folderId ?? null);
      setDocuments(docs);
    } catch (err) {
      console.error('Failed to load documents:', err);
    }
  };

  const handleAddFolder = async () => {
    try {
      const newFolder = await addFolder();
      if (newFolder) {
        await loadFoldersAndDocuments();
      }
    } catch (err) {
      console.error('Error adding watched folder:', err);
    }
  };

  const handleRemoveFolder = async (folderId: number) => {
    try {
      await removeFolder(folderId);
      if (activeFolderId === folderId) {
        setActiveFolderId(null);
      }
      await loadFoldersAndDocuments();
    } catch (err) {
      console.error('Error removing folder:', err);
    }
  };

  const handleRescanFolder = async (folderId: number) => {
    try {
      setScanProgress({ processed: 0, total: 0, current_file: 'Initializing scan...', is_scanning: true });
      await rescanFolder(folderId);
      await loadDocuments(activeFolderId);
    } catch (err) {
      console.error('Error rescanning folder:', err);
    }
  };

  const handleOpenDoc = async (path: string) => {
    try {
      await openDocument(path);
    } catch (err) {
      console.error('Error opening document:', err);
    }
  };

  const handleRemoveDoc = async (docId: number) => {
    try {
      await removeDocumentFromIndex(docId);
      setDocuments((prev) => prev.filter((d) => d.id !== docId));
    } catch (err) {
      console.error('Error removing document from index:', err);
    }
  };

  // Case-insensitive, partial match search filter
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
                ? `Scanning documents... ${scanProgress.processed} files indexed`
                : folders.length === 0
                ? 'No monitored folders'
                : `Ready • ${documents.length} active documents indexed`}
            </span>
          </div>
          {scanProgress.is_scanning && (
            <div style={{ display: 'flex', alignItems: 'center', gap: '8px' }}>
              <span style={{ fontSize: '11px', maxWidth: '200px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                {scanProgress.current_file}
              </span>
              <div className="progress-bar-container">
                <div
                  className="progress-bar-fill"
                  style={{
                    width: scanProgress.total > 0 ? `${(scanProgress.processed / scanProgress.total) * 100}%` : '50%',
                  }}
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
