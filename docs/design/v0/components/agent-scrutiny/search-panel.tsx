"use client";

import { useState } from "react";
import { Search, X, File, ArrowRight } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";

interface SearchResult {
  file: string;
  line: number;
  content: string;
  match: string;
}

const mockResults: SearchResult[] = [
  {
    file: "src/components/Button.tsx",
    line: 8,
    content: 'const buttonVariants = cva(',
    match: "buttonVariants",
  },
  {
    file: "src/components/Button.tsx",
    line: 32,
    content: "interface ButtonProps",
    match: "ButtonProps",
  },
  {
    file: "src/hooks/useAuth.ts",
    line: 5,
    content: "export function useAuth() {",
    match: "useAuth",
  },
  {
    file: "src/components/Input.tsx",
    line: 6,
    content: "export interface InputProps extends InputHTMLAttributes",
    match: "InputProps",
  },
];

export function SearchPanel() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [isSearching, setIsSearching] = useState(false);

  const handleSearch = () => {
    if (!query.trim()) return;
    setIsSearching(true);
    // Simulate search
    setTimeout(() => {
      setResults(mockResults);
      setIsSearching(false);
    }, 300);
  };

  const clearSearch = () => {
    setQuery("");
    setResults([]);
  };

  return (
    <div className="h-full flex flex-col bg-sidebar">
      {/* Header */}
      <div className="p-3 border-b border-sidebar-border">
        <div className="flex items-center gap-2 px-2 py-1.5 bg-input border border-border rounded">
          <Search className="h-4 w-4 text-muted-foreground shrink-0" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSearch()}
            placeholder="Search files (ripgrep)"
            className="flex-1 bg-transparent border-none outline-none text-sm placeholder:text-muted-foreground"
          />
          {query && (
            <Button
              variant="ghost"
              size="sm"
              className="h-5 w-5 p-0"
              onClick={clearSearch}
            >
              <X className="h-3 w-3" />
            </Button>
          )}
        </div>
        <div className="flex items-center gap-2 mt-2 text-xs text-muted-foreground">
          <span>Press Enter to search</span>
          {results.length > 0 && (
            <span className="text-foreground">
              {results.length} results
            </span>
          )}
        </div>
      </div>

      {/* Results */}
      <div className="flex-1 overflow-auto">
        {isSearching ? (
          <div className="flex items-center justify-center h-32 text-muted-foreground text-sm">
            Searching...
          </div>
        ) : results.length > 0 ? (
          <div className="p-2">
            {results.map((result, index) => (
              <button
                key={index}
                type="button"
                className="w-full text-left p-2 rounded hover:bg-sidebar-accent transition-colors group"
              >
                <div className="flex items-center gap-2 mb-1">
                  <File className="h-3.5 w-3.5 text-muted-foreground" />
                  <span className="text-xs font-mono text-primary truncate">
                    {result.file}
                  </span>
                  <span className="text-[10px] text-muted-foreground">
                    :{result.line}
                  </span>
                  <ArrowRight className="h-3 w-3 text-muted-foreground opacity-0 group-hover:opacity-100 ml-auto" />
                </div>
                <pre className="text-xs text-muted-foreground truncate font-mono pl-5">
                  {result.content}
                </pre>
              </button>
            ))}
          </div>
        ) : query ? (
          <div className="flex items-center justify-center h-32 text-muted-foreground text-sm">
            No results found
          </div>
        ) : (
          <div className="flex flex-col items-center justify-center h-32 text-muted-foreground text-sm text-center px-4">
            <Search className="h-8 w-8 mb-2 opacity-50" />
            <p>Search across all files</p>
            <p className="text-xs mt-1">Powered by ripgrep</p>
          </div>
        )}
      </div>
    </div>
  );
}
