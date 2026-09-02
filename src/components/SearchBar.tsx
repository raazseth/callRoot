import React from 'react';
import { Search } from 'lucide-react';

interface SearchBarProps {
  query: string;
  onQueryChange: (q: string) => void;
  resultCount: number;
}

export const SearchBar: React.FC<SearchBarProps> = ({ query, onQueryChange, resultCount }) => {
  return (
    <header className="top-header">
      <div className="search-container">
        <Search size={16} className="search-icon" />
        <input
          type="text"
          className="search-input"
          placeholder="Search documents by filename..."
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
        />
      </div>
      <div className="header-meta">
        <span>{resultCount} {resultCount === 1 ? 'document' : 'documents'} found</span>
      </div>
    </header>
  );
};
