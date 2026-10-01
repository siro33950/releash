import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { subscribeState } from "@/lib/client";
import { CreateWorktreeModal } from "./CreateWorktreeModal";

it("Load moreの再購読中もタスク行を表示し新しい一覧で置き換える", async () => {
	vi.clearAllMocks();
	const requests = new Map<string, Parameters<typeof subscribeState>[1]>();
	vi.mocked(subscribeState).mockImplementation((target, receive) => {
		const kind = typeof target === "string" ? target : target.kind;
		if (kind === "notion-tasks" && typeof target !== "string")
			requests.set(target.args[1], receive);
		else if (kind === "notion-label-options") receive({ options: [] });
		else if (kind === "issues") receive({ issues: [] });
		else if (kind === "branches") receive([{ name: "main", is_remote: false }]);
		else receive([]);
		return vi.fn();
	});
	const user = userEvent.setup();
	render(
		<CreateWorktreeModal
			open
			repoPaths={["/repo"]}
			onCreated={vi.fn()}
			onClose={vi.fn()}
		/>,
	);
	await user.click(screen.getByRole("tab", { name: /Notion/ }));
	const task = {
		id: "one",
		title: "First task",
		url: "",
		labels: {},
		branch_name: "feat/first",
		created_at: "",
		last_edited_at: "",
	};
	act(() => requests.get("20")?.({ page: { tasks: [task], has_more: true } }));
	expect(screen.getByRole("button", { name: /First task/ })).toBeVisible();
	await user.click(screen.getByRole("button", { name: /Load more/ }));
	expect(subscribeState).toHaveBeenCalledWith(
		{ kind: "notion-tasks", args: ["/repo", "40"] },
		expect.any(Function),
		expect.any(Function),
	);
	expect(screen.getByRole("button", { name: /First task/ })).toBeVisible();
	expect(screen.getByRole("button", { name: /Load more/ })).toBeDisabled();
	act(() =>
		requests.get("40")?.({
			page: {
				tasks: [{ ...task, id: "two", title: "Replacement task" }],
				has_more: false,
			},
		}),
	);
	expect(
		screen.queryByRole("button", { name: /First task/ }),
	).not.toBeInTheDocument();
	expect(
		screen.getByRole("button", { name: /Replacement task/ }),
	).toBeVisible();
});
