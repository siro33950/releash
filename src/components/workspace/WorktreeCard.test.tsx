import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { BranchCard as BranchCardType } from "@/types/git";
import { BranchCard } from "./WorktreeCard";

vi.mock("@tauri-apps/plugin-opener", () => ({
	openUrl: vi.fn(),
}));

function makeBranch(overrides: Partial<BranchCardType> = {}): BranchCardType {
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
		is_remote_only: false,
		has_upstream: true,
		...overrides,
	};
}

describe("BranchCard", () => {
	const noop = vi.fn();

	it("ahead/behind が 0 のとき表示しない", () => {
		render(<BranchCard branch={makeBranch()} onOpen={noop} onDelete={noop} />);
		expect(screen.queryByText(/↑/)).not.toBeInTheDocument();
		expect(screen.queryByText(/↓/)).not.toBeInTheDocument();
	});

	it("ahead > 0 のとき ↑N を表示", () => {
		render(
			<BranchCard
				branch={makeBranch({ ahead: 3, behind: 0 })}
				onOpen={noop}
				onDelete={noop}
			/>,
		);
		expect(screen.getByText("↑3")).toBeInTheDocument();
		expect(screen.queryByText(/↓/)).not.toBeInTheDocument();
	});

	it("behind > 0 のとき ↓M を表示", () => {
		render(
			<BranchCard
				branch={makeBranch({ ahead: 0, behind: 2 })}
				onOpen={noop}
				onDelete={noop}
			/>,
		);
		expect(screen.getByText("↓2")).toBeInTheDocument();
		expect(screen.queryByText(/↑/)).not.toBeInTheDocument();
	});

	it("ahead と behind の両方があるとき ↑N ↓M を表示", () => {
		render(
			<BranchCard
				branch={makeBranch({ ahead: 5, behind: 3 })}
				onOpen={noop}
				onDelete={noop}
			/>,
		);
		const text = screen.getByText(/↑5/);
		expect(text).toBeInTheDocument();
		expect(text.textContent).toContain("↓3");
	});

	it("is_remote_only のとき Globe アイコンを使用", () => {
		const { container } = render(
			<BranchCard
				branch={makeBranch({ is_remote_only: true })}
				onOpen={noop}
				onDelete={noop}
			/>,
		);
		expect(container.querySelector(".lucide-globe")).toBeInTheDocument();
		expect(
			container.querySelector(".lucide-git-branch"),
		).not.toBeInTheDocument();
	});

	it("upstream ありのとき GitBranch アイコンを使用", () => {
		const { container } = render(
			<BranchCard branch={makeBranch()} onOpen={noop} onDelete={noop} />,
		);
		expect(container.querySelector(".lucide-git-branch")).toBeInTheDocument();
	});

	it("upstream なしのとき Monitor アイコンを使用", () => {
		const { container } = render(
			<BranchCard
				branch={makeBranch({ has_upstream: false })}
				onOpen={noop}
				onDelete={noop}
			/>,
		);
		expect(container.querySelector(".lucide-monitor")).toBeInTheDocument();
		expect(
			container.querySelector(".lucide-git-branch"),
		).not.toBeInTheDocument();
	});
});
