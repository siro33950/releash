import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { TabBarContainer, TabBarItem } from "./tab-bar";

describe("TabBarContainer", () => {
	it("role=tablist でレンダリングされる", () => {
		render(
			<TabBarContainer ariaLabel="Test tabs">
				<div>child</div>
			</TabBarContainer>,
		);
		const tablist = screen.getByRole("tablist");
		expect(tablist).toBeInTheDocument();
		expect(tablist).toHaveAttribute("aria-orientation", "horizontal");
		expect(tablist).toHaveAttribute("aria-label", "Test tabs");
	});

	it("追加の className を適用できる", () => {
		render(
			<TabBarContainer className="custom-class">
				<div>child</div>
			</TabBarContainer>,
		);
		expect(screen.getByRole("tablist")).toHaveClass("custom-class");
	});
});

describe("TabBarItem", () => {
	it("アクティブ時に aria-selected=true", () => {
		render(
			<TabBarItem isActive={true} onClick={() => {}}>
				Tab 1
			</TabBarItem>,
		);
		const tab = screen.getByRole("tab");
		expect(tab).toHaveAttribute("aria-selected", "true");
		expect(tab).toHaveAttribute("tabindex", "0");
	});

	it("非アクティブ時に aria-selected=false, tabindex=-1", () => {
		render(
			<TabBarItem isActive={false} onClick={() => {}}>
				Tab 1
			</TabBarItem>,
		);
		const tab = screen.getByRole("tab");
		expect(tab).toHaveAttribute("aria-selected", "false");
		expect(tab).toHaveAttribute("tabindex", "-1");
	});

	it("クリックで onClick が呼ばれる", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		render(
			<TabBarItem isActive={false} onClick={onClick}>
				Tab 1
			</TabBarItem>,
		);
		await user.click(screen.getByRole("tab"));
		expect(onClick).toHaveBeenCalledOnce();
	});

	it("Enter キーで onClick が呼ばれる", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		render(
			<TabBarItem isActive={true} onClick={onClick}>
				Tab 1
			</TabBarItem>,
		);
		screen.getByRole("tab").focus();
		await user.keyboard("{Enter}");
		expect(onClick).toHaveBeenCalledOnce();
	});

	it("Space キーで onClick が呼ばれる", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		render(
			<TabBarItem isActive={true} onClick={onClick}>
				Tab 1
			</TabBarItem>,
		);
		screen.getByRole("tab").focus();
		await user.keyboard(" ");
		expect(onClick).toHaveBeenCalledOnce();
	});

	it("onClose が渡されると閉じるボタンが表示される", () => {
		render(
			<TabBarItem
				isActive={true}
				onClick={() => {}}
				onClose={() => {}}
				closeLabel="Close Tab 1"
			>
				Tab 1
			</TabBarItem>,
		);
		expect(screen.getByLabelText("Close Tab 1")).toBeInTheDocument();
	});

	it("onClose が未指定のとき閉じるボタンが非表示", () => {
		render(
			<TabBarItem isActive={true} onClick={() => {}}>
				Tab 1
			</TabBarItem>,
		);
		expect(screen.queryByRole("button")).not.toBeInTheDocument();
	});

	it("閉じるボタンで onClose が呼ばれ、onClick は呼ばれない", async () => {
		const user = userEvent.setup();
		const onClick = vi.fn();
		const onClose = vi.fn();
		render(
			<TabBarItem
				isActive={true}
				onClick={onClick}
				onClose={onClose}
				closeLabel="Close Tab 1"
			>
				Tab 1
			</TabBarItem>,
		);
		await user.click(screen.getByLabelText("Close Tab 1"));
		expect(onClose).toHaveBeenCalledOnce();
		expect(onClick).not.toHaveBeenCalled();
	});

	it("カスタム onKeyDown が呼ばれる", async () => {
		const user = userEvent.setup();
		const onKeyDown = vi.fn();
		render(
			<TabBarItem isActive={true} onClick={() => {}} onKeyDown={onKeyDown}>
				Tab 1
			</TabBarItem>,
		);
		screen.getByRole("tab").focus();
		await user.keyboard("{ArrowRight}");
		expect(onKeyDown).toHaveBeenCalledOnce();
	});

	it("aria-controls と id が設定される", () => {
		render(
			<TabBarItem
				isActive={true}
				onClick={() => {}}
				id="tab-1"
				ariaControls="panel-1"
			>
				Tab 1
			</TabBarItem>,
		);
		const tab = screen.getByRole("tab");
		expect(tab).toHaveAttribute("id", "tab-1");
		expect(tab).toHaveAttribute("aria-controls", "panel-1");
	});

	it("追加の HTML 属性がスプレッドされる", () => {
		render(
			<TabBarItem
				isActive={false}
				onClick={() => {}}
				data-testid="custom-tab"
				draggable={true}
			>
				Tab 1
			</TabBarItem>,
		);
		const tab = screen.getByTestId("custom-tab");
		expect(tab).toHaveAttribute("draggable", "true");
	});
});
