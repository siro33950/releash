import { useCallback, useEffect, useRef, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import type { Subscribe } from "./useMessageBus";

interface FileContent {
  path: string;
  original: string;
  modified: string;
  staged: string | null;
}

export type DiffBase = "HEAD" | "staged";

interface UseRemoteFileContentOptions {
  subscribe: Subscribe;
  send: (msg: WsMessage) => void;
}

export function useRemoteFileContent({
  subscribe,
  send,
}: UseRemoteFileContentOptions) {
  const [content, setContent] = useState<FileContent | null>(null);
  const [loading, setLoading] = useState(false);
  const currentPathRef = useRef<string | null>(null);
  const currentDiffBaseRef = useRef<DiffBase>("HEAD");

  const requestContent = useCallback(
    (path: string, diffBase?: DiffBase) => {
      currentPathRef.current = path;
      if (diffBase !== undefined) {
        currentDiffBaseRef.current = diffBase;
      }
      setLoading(true);
      send({
        type: "file_content_request",
        payload: { path, diff_base: currentDiffBaseRef.current },
      });
    },
    [send],
  );

  useEffect(() => {
    return subscribe((msg: WsMessage) => {
      if (msg.type === "file_content_response") {
        if (msg.payload.path === currentPathRef.current) {
          setContent({
            path: msg.payload.path,
            original: msg.payload.original,
            modified: msg.payload.modified,
            staged: msg.payload.staged ?? null,
          });
          setLoading(false);
        }
      } else if (msg.type === "file_change") {
        if (msg.payload.path === currentPathRef.current) {
          send({
            type: "file_content_request",
            payload: {
              path: currentPathRef.current,
              diff_base: currentDiffBaseRef.current,
            },
          });
        }
      } else if (msg.type === "git_stage_result" && msg.payload.success) {
        if (currentPathRef.current) {
          send({
            type: "file_content_request",
            payload: {
              path: currentPathRef.current,
              diff_base: currentDiffBaseRef.current,
            },
          });
        }
      }
    });
  }, [subscribe, send]);

  const clear = useCallback(() => {
    currentPathRef.current = null;
    currentDiffBaseRef.current = "HEAD";
    setContent(null);
    setLoading(false);
  }, []);

  return { content, loading, requestContent, clear };
}
