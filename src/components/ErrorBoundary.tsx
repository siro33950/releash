import { Component, type ErrorInfo, type ReactNode } from "react";
import { reportFrontendError } from "@/lib/telemetry";

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
				className="px-4 py-2 text-sm rounded bg-primary text-primary-foreground hover:opacity-90"
			>
				Retry
			</button>
		</div>
	);
}

interface FrontendErrorBoundaryProps {
	children: ReactNode;
}

interface FrontendErrorBoundaryState {
	error: Error | null;
}

export class FrontendErrorBoundary extends Component<
	FrontendErrorBoundaryProps,
	FrontendErrorBoundaryState
> {
	state: FrontendErrorBoundaryState = { error: null };

	static getDerivedStateFromError(error: Error): FrontendErrorBoundaryState {
		return { error };
	}

	componentDidCatch(error: Error, info: ErrorInfo): void {
		reportFrontendError(error, "react_error", info.componentStack ?? undefined);
	}

	reset = (): void => {
		this.setState({ error: null });
	};

	render() {
		const { error } = this.state;
		if (error) {
			return <FallbackUI error={error} resetError={this.reset} />;
		}
		return this.props.children;
	}
}
