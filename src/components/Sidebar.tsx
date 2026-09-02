import React from 'react';
import { Folder, FolderPlus, RefreshCw, Trash2, HardDrive, Files } from 'lucide-react';
import { WatchedFolder } from '../types';

interface SidebarProps {
  folders: WatchedFolder[];
  activeFolderId: number | null;
  onSelectFolder: (id: number | null) => void;
  onAddFolder: () => void;
  onRescanFolder: (id: number) => void;
  onRemoveFolder: (id: number) => void;
  totalDocumentsCount: number;
}

export const Sidebar: React.FC<SidebarProps> = ({
  folders,
  activeFolderId,
  onSelectFolder,
  onAddFolder,
  onRescanFolder,
  onRemoveFolder,
  totalDocumentsCount,
}) => {
  const getFolderName = (path: string) => {
    const parts = path.split(/[/\\]/).filter(Boolean);
    return parts[parts.length - 1] || path;
  };

  return (
    <aside className="sidebar">
      <div className="sidebar-header">
        <div className="brand-logo">R</div>
        <div className="brand-title">Root</div>
      </div>

      <div className="sidebar-actions">
        <button className="btn-add-folder" onClick={onAddFolder}>
          <FolderPlus size={16} />
          Add Folder
        </button>
      </div>

      <div className="sidebar-section-title">Library</div>
      <div className="folder-list" style={{ flex: '0 0 auto' }}>
        <div
          className={`folder-item ${activeFolderId === null ? 'active' : ''}`}
          onClick={() => onSelectFolder(null)}
        >
          <div className="folder-info">
            <Files size={16} />
            <span className="folder-name">All Documents ({totalDocumentsCount})</span>
          </div>
        </div>
      </div>

      <div className="sidebar-section-title">Watched Folders ({folders.length})</div>
      <div className="folder-list">
        {folders.map((folder) => (
          <div
            key={folder.id}
            className={`folder-item ${activeFolderId === folder.id ? 'active' : ''}`}
            onClick={() => onSelectFolder(folder.id)}
          >
            <div className="folder-info" title={folder.path}>
              <Folder size={16} />
              <span className="folder-name">{getFolderName(folder.path)}</span>
            </div>
            <div className="folder-actions" onClick={(e) => e.stopPropagation()}>
              <button
                className="btn-icon"
                title="Rescan folder"
                onClick={() => onRescanFolder(folder.id)}
              >
                <RefreshCw size={13} />
              </button>
              <button
                className="btn-icon danger"
                title="Remove folder"
                onClick={() => onRemoveFolder(folder.id)}
              >
                <Trash2 size={13} />
              </button>
            </div>
          </div>
        ))}
      </div>
    </aside>
  );
};
