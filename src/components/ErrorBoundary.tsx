import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props {
	children: ReactNode;
	onRetry?: () => void;
}

interface State {
	hasError: boolean;
	error: Error | null;
}

export class WorktreeErrorBoundary extends Component<Props, State> {
	constructor(props: Props) {
		super(props);
		this.state = { hasError: false, error: null };
	}

	static getDerivedStateFromError(error: Error): State {
		return { hasError: true, error };
	}

	componentDidCatch(error: Error, info: ErrorInfo) {
		console.error("WorktreeErrorBoundary caught error:", error, info);
	}

	private handleRetry = () => {
		this.props.onRetry?.();
		this.setState({ hasError: false, error: null });
	};

	render() {
		if (this.state.hasError) {
			return (
				<div className="flex flex-col items-center justify-center h-full gap-4 p-8 text-muted-foreground">
					<p className="text-sm">
						ビューの描画中にエラーが発生しました
					</p>
					<p className="text-xs text-muted-foreground/60 max-w-md text-center">
						{this.state.error?.message}
					</p>
					<button
						type="button"
						onClick={this.handleRetry}
						className="px-3 py-1.5 text-xs rounded bg-primary text-primary-foreground hover:bg-primary/90 transition-colors"
					>
						再試行
					</button>
				</div>
			);
		}
		return this.props.children;
	}
}
