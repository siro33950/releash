import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { TooltipProvider } from "@/components/ui/tooltip";
import type { WorktreeBranch } from "@/types/git";
import { BranchCard } from "./WorktreeCard";

vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: vi.fn(),
}));

function makeBranch(overrides: Partial<WorktreeBranch> = {}): WorktreeBranch {
	return {
		name: "feat/test",
		is_default: false,
		worktree_path: null,
		dirty_count: 0,
		is_merged: false,
		has_pr: false,
		pr_number: null,
		pr_url: null,
		ahead: 0,
		behind: 0,
		has_upstream: true,
		base_ahead: 0,
		...overrides,
	};
}

describe("BranchCard", () => {
	const noop = vi.fn();

	it("ahead/behind が 0 のとき表示しない", () => {
		render(
			<TooltipProvider>
				<BranchCard branch={makeBranch()} onOpen={noop} onDelete={noop} />
			</TooltipProvider>,
		);
		expect(screen.queryByText(/↑/)).not.toBeInTheDocument();
		expect(screen.queryByText(/↓/)).not.toBeInTheDocument();
	});

	it("ahead > 0 のとき ↑N を表示", () => {
		render(
			<TooltipProvider>
				<BranchCard
					branch={makeBranch({ ahead: 3, behind: 0 })}
					onOpen={noop}
					onDelete={noop}
				/>
			</TooltipProvider>,
		);
		expect(screen.getByText("↑3")).toBeInTheDocument();
		expect(screen.queryByText(/↓/)).not.toBeInTheDocument();
	});

	it("behind > 0 のとき ↓M を表示", () => {
		render(
			<TooltipProvider>
				<BranchCard
					branch={makeBranch({ ahead: 0, behind: 2 })}
					onOpen={noop}
					onDelete={noop}
				/>
			</TooltipProvider>,
		);
		expect(screen.getByText("↓2")).toBeInTheDocument();
		expect(screen.queryByText(/↑/)).not.toBeInTheDocument();
	});

	it("ahead と behind の両方があるとき ↑N ↓M を表示", () => {
		render(
			<TooltipProvider>
				<BranchCard
					branch={makeBranch({ ahead: 5, behind: 3 })}
					onOpen={noop}
					onDelete={noop}
				/>
			</TooltipProvider>,
		);
		const text = screen.getByText(/↑5/);
		expect(text).toBeInTheDocument();
		expect(text.textContent).toContain("↓3");
	});

	it("upstream ありのとき GitBranch アイコンを使用", () => {
		const { container } = render(
			<TooltipProvider>
				<BranchCard branch={makeBranch()} onOpen={noop} onDelete={noop} />
			</TooltipProvider>,
		);
		expect(container.querySelector(".lucide-git-branch")).toBeInTheDocument();
	});

	it("upstream なしのとき Monitor アイコンを使用", () => {
		const { container } = render(
			<TooltipProvider>
				<BranchCard
					branch={makeBranch({ has_upstream: false })}
					onOpen={noop}
					onDelete={noop}
				/>
			</TooltipProvider>,
		);
		expect(container.querySelector(".lucide-monitor")).toBeInTheDocument();
		expect(
			container.querySelector(".lucide-git-branch"),
		).not.toBeInTheDocument();
	});
});
