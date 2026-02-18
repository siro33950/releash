import * as Sentry from "@sentry/react";
import type { ReactNode } from "react";

function FallbackUI({
	error,
	resetError,
}: {
	error: Error;
	resetError: () => void;
}) {
	return (
		<div className="flex flex-col items-center justify-center h-screen gap-4 bg-background text-foreground p-8">
			<h1 className="text-lg font-semibold">Something went wrong</h1>
			<p className="text-sm text-muted-foreground max-w-md text-center">
				{error.message}
			</p>
			<button
				type="button"
				onClick={resetError}
				className="px-4 py-2 text-sm rounded bg-accent text-accent-foreground hover:opacity-90"
			>
				Retry
			</button>
		</div>
	);
}

export function SentryErrorBoundary({ children }: { children: ReactNode }) {
	return (
		<Sentry.ErrorBoundary
			fallback={({ error, resetError }) => (
				<FallbackUI error={error as Error} resetError={resetError} />
			)}
		>
			{children}
		</Sentry.ErrorBoundary>
	);
}
