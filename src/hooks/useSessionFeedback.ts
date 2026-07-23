import { useCallback, useEffect, useState } from "react";
import {
	dismissAgentSessionFeedback,
	listAgentSessionFeedback,
	retryAgentSessionFeedback,
	type SessionFeedbackEntry,
} from "@/hooks/useSessionStore";

const FEEDBACK_PAGE_LIMIT = 32;
const MAX_MIRRORED_FEEDBACK = 512;

interface FeedbackMirror {
	sessionId: string | null;
	entries: SessionFeedbackEntry[];
	nextCursor: string | null;
}

function actionIdentity(
	entry: SessionFeedbackEntry,
	action: "dismiss" | "retry_resolution",
): string {
	const identity = entry.action_identities.find(
		(candidate) =>
			candidate.action === action &&
			candidate.origin_revision === entry.revision,
	);
	if (!identity) {
		throw new Error(`Feedback ${action} action is unavailable`);
	}
	return identity.action_id;
}

/** Bounded UI mirror of backend-owned, identity-keyed feedback pages. */
export function useSessionFeedback(sessionId: string | null) {
	const [mirror, setMirror] = useState<FeedbackMirror>({
		sessionId: null,
		entries: [],
		nextCursor: null,
	});
	const entries = mirror.sessionId === sessionId ? mirror.entries : [];
	const nextCursor = mirror.sessionId === sessionId ? mirror.nextCursor : null;

	const refresh = useCallback(async () => {
		if (!sessionId) {
			setMirror({ sessionId: null, entries: [], nextCursor: null });
			return;
		}
		const page = await listAgentSessionFeedback(
			sessionId,
			FEEDBACK_PAGE_LIMIT,
			null,
		);
		setMirror({
			sessionId,
			entries: page.entries.slice(0, MAX_MIRRORED_FEEDBACK),
			nextCursor: page.next_cursor,
		});
	}, [sessionId]);

	useEffect(() => {
		let cancelled = false;
		if (!sessionId) {
			setMirror({ sessionId: null, entries: [], nextCursor: null });
			return;
		}
		void listAgentSessionFeedback(sessionId, FEEDBACK_PAGE_LIMIT, null)
			.then((page) => {
				if (!cancelled) {
					setMirror({
						sessionId,
						entries: page.entries.slice(0, MAX_MIRRORED_FEEDBACK),
						nextCursor: page.next_cursor,
					});
				}
			})
			.catch(() => {
				// Keep the last successful mirror. A query failure is not evidence that
				// backend-owned unresolved feedback became empty.
			});
		return () => {
			cancelled = true;
		};
	}, [sessionId]);

	const loadNextPage = useCallback(async () => {
		if (!sessionId || !nextCursor || entries.length >= MAX_MIRRORED_FEEDBACK) {
			return;
		}
		const requestedCursor = nextCursor;
		const page = await listAgentSessionFeedback(
			sessionId,
			FEEDBACK_PAGE_LIMIT,
			requestedCursor,
		);
		setMirror((current) => {
			if (
				current.sessionId !== sessionId ||
				current.nextCursor !== requestedCursor
			) {
				return current;
			}
			const known = new Set(current.entries.map((entry) => entry.feedback_id));
			const appended = page.entries.filter(
				(entry) => !known.has(entry.feedback_id),
			);
			const merged = [...current.entries, ...appended].slice(
				0,
				MAX_MIRRORED_FEEDBACK,
			);
			return {
				sessionId,
				entries: merged,
				nextCursor:
					merged.length >= MAX_MIRRORED_FEEDBACK ? null : page.next_cursor,
			};
		});
	}, [entries.length, nextCursor, sessionId]);

	const dismiss = useCallback(
		async (entry: SessionFeedbackEntry) => {
			if (!sessionId || entry.session_id !== sessionId) return;
			await dismissAgentSessionFeedback(
				sessionId,
				entry.feedback_id,
				entry.revision,
				actionIdentity(entry, "dismiss"),
			);
			setMirror((current) =>
				current.sessionId === sessionId
					? {
							...current,
							entries: current.entries.filter(
								(candidate) => candidate.feedback_id !== entry.feedback_id,
							),
						}
					: current,
			);
		},
		[sessionId],
	);

	const retry = useCallback(
		async (entry: SessionFeedbackEntry) => {
			if (!sessionId || entry.session_id !== sessionId) return;
			const outcome = await retryAgentSessionFeedback(
				sessionId,
				entry.feedback_id,
				entry.revision,
				actionIdentity(entry, "retry_resolution"),
			);
			setMirror((current) => {
				if (current.sessionId !== sessionId) return current;
				if (outcome.type === "resolved") {
					return {
						...current,
						entries: current.entries.filter(
							(candidate) => candidate.feedback_id !== entry.feedback_id,
						),
					};
				}
				return {
					...current,
					entries: current.entries.map((candidate) =>
						candidate.feedback_id === outcome.entry.feedback_id
							? outcome.entry
							: candidate,
					),
				};
			});
		},
		[sessionId],
	);

	return {
		entries,
		dismiss,
		retry,
		refresh,
		hasMore: nextCursor !== null,
		loadNextPage,
	};
}
