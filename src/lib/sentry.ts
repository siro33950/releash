import * as Sentry from "@sentry/react";

const SENTRY_DSN = import.meta.env.VITE_SENTRY_DSN as string | undefined;

const USERNAME_PATTERN = /\/(Users|home|Documents and Settings)\/[^/]+\//g;

export function initSentry(enabled: boolean): void {
	if (!SENTRY_DSN) {
		return;
	}

	Sentry.init({
		dsn: SENTRY_DSN,
		enabled,
		sendDefaultPii: false,
		tracesSampleRate: 0,
		replaysSessionSampleRate: 0,
		replaysOnErrorSampleRate: 0,
		autoSessionTracking: true,
		beforeSend(event) {
			return scrubPaths(event);
		},
	});
}

export function setSentryEnabled(enabled: boolean): void {
	const client = Sentry.getClient();
	if (client) {
		client.getOptions().enabled = enabled;
	}
}

function scrubPaths(event: Sentry.ErrorEvent): Sentry.ErrorEvent {
	if (event.exception?.values) {
		for (const ex of event.exception.values) {
			if (ex.stacktrace?.frames) {
				for (const frame of ex.stacktrace.frames) {
					if (frame.filename) {
						frame.filename = frame.filename.replace(USERNAME_PATTERN, "/$1/~/");
					}
					if (frame.abs_path) {
						frame.abs_path = frame.abs_path.replace(USERNAME_PATTERN, "/$1/~/");
					}
				}
			}
		}
	}
	return event;
}
