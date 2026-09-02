export interface WatchedFolder {
  id: number;
  path: string;
  created_at: number;
  updated_at: number;
}

export type DocumentStatus = 'ACTIVE' | 'MISSING' | 'IGNORED';

export interface DocumentRecord {
  id: number;
  watched_folder_id: number;
  path: string;
  filename: string;
  extension: string;
  size: number;
  created_at: number;
  modified_at: number;
  indexed_at: number;
  status: DocumentStatus;
}

export interface ScanProgress {
  processed: number;
  total: number;
  current_file: string;
  is_scanning: boolean;
}

export type SortColumn = 'filename' | 'path' | 'modified_at' | 'size' | 'extension';
export type SortDirection = 'asc' | 'desc';
