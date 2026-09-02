import React, { useState, useMemo } from 'react';
import { ArrowUpDown, ArrowUp, ArrowDown, ExternalLink, Trash2, FileText } from 'lucide-react';
import { DocumentRecord, SortColumn, SortDirection } from '../types';

interface DocumentListProps {
  documents: DocumentRecord[];
  onOpenDoc: (path: string) => void;
  onRemoveDoc: (id: number) => void;
}

export const DocumentList: React.FC<DocumentListProps> = ({ documents, onOpenDoc, onRemoveDoc }) => {
  const [sortColumn, setSortColumn] = useState<SortColumn>('modified_at');
  const [sortDirection, setSortDirection] = useState<SortDirection>('desc');

  const handleSort = (col: SortColumn) => {
    if (sortColumn === col) {
      setSortDirection(sortDirection === 'asc' ? 'desc' : 'asc');
    } else {
      setSortColumn(col);
      setSortDirection('asc');
    }
  };

  const sortedDocuments = useMemo(() => {
    return [...documents].sort((a, b) => {
      let valA = a[sortColumn];
      let valB = b[sortColumn];

      if (typeof valA === 'string') {
        valA = (valA as string).toLowerCase();
        valB = (valB as string).toLowerCase();
      }

      if (valA < valB) return sortDirection === 'asc' ? -1 : 1;
      if (valA > valB) return sortDirection === 'asc' ? 1 : -1;
      return 0;
    });
  }, [documents, sortColumn, sortDirection]);

  const formatSize = (bytes: number): string => {
    if (bytes === 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  };

  const formatDate = (timestamp: number): string => {
    if (!timestamp) return '-';
    const date = new Date(timestamp * 1000);
    return date.toLocaleDateString(undefined, {
      year: 'numeric',
      month: 'short',
      day: 'numeric',
      hour: '2-digit',
      minute: '2-digit',
    });
  };

  const getBadgeClass = (ext: string) => {
    const cleanExt = ext.toLowerCase().replace('.', '');
    if (['pdf'].includes(cleanExt)) return 'pdf';
    if (['docx', 'doc'].includes(cleanExt)) return 'docx';
    if (['txt'].includes(cleanExt)) return 'txt';
    if (['md', 'markdown'].includes(cleanExt)) return 'md';
    if (['xlsx', 'csv'].includes(cleanExt)) return 'xlsx';
    return 'other';
  };

  const renderSortIcon = (col: SortColumn) => {
    if (sortColumn !== col) return <ArrowUpDown size={12} style={{ opacity: 0.4 }} />;
    return sortDirection === 'asc' ? <ArrowUp size={12} /> : <ArrowDown size={12} />;
  };

  return (
    <div className="document-table-container">
      <table className="document-table">
        <thead>
          <tr>
            <th onClick={() => handleSort('filename')}>
              <div className="th-content">
                Filename {renderSortIcon('filename')}
              </div>
            </th>
            <th onClick={() => handleSort('path')}>
              <div className="th-content">
                Folder {renderSortIcon('path')}
              </div>
            </th>
            <th onClick={() => handleSort('modified_at')}>
              <div className="th-content">
                Modified Date {renderSortIcon('modified_at')}
              </div>
            </th>
            <th onClick={() => handleSort('size')}>
              <div className="th-content">
                File Size {renderSortIcon('size')}
              </div>
            </th>
            <th onClick={() => handleSort('extension')}>
              <div className="th-content">
                Type {renderSortIcon('extension')}
              </div>
            </th>
            <th>Actions</th>
          </tr>
        </thead>
        <tbody>
          {sortedDocuments.map((doc) => {
            const lastSlash = Math.max(doc.path.lastIndexOf('/'), doc.path.lastIndexOf('\\'));
            const folderPath = lastSlash > 0 ? doc.path.substring(0, lastSlash) : doc.path;
            const extClean = doc.extension.toLowerCase().replace('.', '');

            return (
              <tr key={doc.id} className="document-row">
                <td className="cell-name" title={doc.path}>
                  <FileText size={16} style={{ color: 'var(--text-dim)', flexShrink: 0 }} />
                  <span>{doc.filename}</span>
                </td>
                <td title={folderPath} style={{ maxWidth: '240px', overflow: 'hidden', textOverflow: 'ellipsis' }}>
                  {folderPath}
                </td>
                <td>{formatDate(doc.modified_at)}</td>
                <td>{formatSize(doc.size)}</td>
                <td>
                  <span className={`ext-badge ${getBadgeClass(extClean)}`}>
                    {extClean || 'file'}
                  </span>
                </td>
                <td>
                  <div className="cell-actions">
                    <button
                      className="btn-open"
                      onClick={() => onOpenDoc(doc.path)}
                      title="Open file in default OS application"
                    >
                      <ExternalLink size={12} style={{ display: 'inline', marginRight: '4px' }} />
                      Open
                    </button>
                    <button
                      className="btn-icon danger"
                      onClick={() => onRemoveDoc(doc.id)}
                      title="Remove from Root index"
                    >
                      <Trash2 size={13} />
                    </button>
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
};
