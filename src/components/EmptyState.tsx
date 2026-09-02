import React from 'react';
import { FolderPlus, FolderSearch } from 'lucide-react';

interface EmptyStateProps {
  onAddFolder: () => void;
}

export const EmptyState: React.FC<EmptyStateProps> = ({ onAddFolder }) => {
  return (
    <div className="empty-state-container">
      <div className="empty-state-card">
        <div className="empty-state-icon">
          <FolderSearch size={32} />
        </div>
        <h2 className="empty-state-title">Welcome to Root</h2>
        <p className="empty-state-description">
          No folders are being watched yet. Choose local folders to continuously monitor and index your documents automatically.
        </p>
        <button className="btn-add-folder" onClick={onAddFolder}>
          <FolderPlus size={18} />
          Add Folder to Watch
        </button>
      </div>
    </div>
  );
};
