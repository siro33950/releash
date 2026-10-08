import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, describe, expect, it, vi } from "vitest";
import type { NotionRepoDraft } from "@/hooks/useNotionSettings";
import { NotionSettingsSection } from "./NotionSettingsSection";

beforeAll(() => {
	HTMLElement.prototype.hasPointerCapture = vi.fn() as never;
	HTMLElement.prototype.releasePointerCapture = vi.fn() as never;
	HTMLElement.prototype.setPointerCapture = vi.fn() as never;
	HTMLElement.prototype.scrollIntoView = vi.fn() as never;
});

function makeDraft(overrides: Partial<NotionRepoDraft> = {}): NotionRepoDraft {
	return {
		apiToken: "",
		databaseId: "",
		propertyMapping: {
			title: "Name",
			labels: [],
			branch_name: "",
			branch_prefix: "",
		},
		validating: false,
		validationStatus: null,
		properties: [],
		markedForDelete: false,
		...overrides,
	};
}

describe("NotionSettingsSection", () => {
	it("should show empty message when no repos", () => {
		render(
			<NotionSettingsSection
				repoPaths={[]}
				drafts={new Map()}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		expect(screen.getByText("No repositories registered.")).toBeInTheDocument();
	});

	it("should render a config form for each repo", () => {
		const drafts = new Map([
			["/repo/alpha", makeDraft()],
			["/repo/beta", makeDraft()],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/alpha", "/repo/beta"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		expect(screen.getByText("alpha")).toBeInTheDocument();
		expect(screen.getByText("beta")).toBeInTheDocument();
		expect(screen.getAllByText("API Token")).toHaveLength(2);
	});

	it("should call updateDraft when api token changes", async () => {
		const user = userEvent.setup();
		const updateDraft = vi.fn();
		const drafts = new Map([["/repo/a", makeDraft()]]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={updateDraft}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		const tokenInput = screen.getByLabelText("API Token");
		await user.type(tokenInput, "t");

		expect(updateDraft).toHaveBeenCalledWith("/repo/a", expect.any(Function));
	});

	it("should call validate when Test Connection is clicked", async () => {
		const user = userEvent.setup();
		const validate = vi.fn();
		const drafts = new Map([
			["/repo/a", makeDraft({ apiToken: "token", databaseId: "db-id" })],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={validate}
				markForDelete={vi.fn()}
			/>,
		);

		await user.click(screen.getByText("Test Connection"));
		expect(validate).toHaveBeenCalledWith("/repo/a");
	});

	it("should disable Test Connection when token or db is empty", () => {
		const drafts = new Map([["/repo/a", makeDraft()]]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		expect(
			screen.getByText("Test Connection").closest("button"),
		).toBeDisabled();
	});

	it("should show success message when validation succeeds", () => {
		const drafts = new Map([
			[
				"/repo/a",
				makeDraft({
					apiToken: "token",
					databaseId: "db-id",
					validationStatus: "success",
					properties: [{ name: "Name", property_type: "title", options: [] }],
				}),
			],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		expect(screen.getByText("Connection successful")).toBeInTheDocument();
		expect(screen.getByText("Property Mapping")).toBeInTheDocument();
	});

	it("should show error message when validation fails", () => {
		const drafts = new Map([
			["/repo/a", makeDraft({ validationStatus: "Invalid API token" })],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		expect(screen.getByText("Invalid API token")).toBeInTheDocument();
	});

	it("should show delete confirmation when marked for delete", () => {
		const drafts = new Map([
			[
				"/repo/a",
				makeDraft({
					apiToken: "token",
					databaseId: "db-id",
					markedForDelete: true,
				}),
			],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		expect(
			screen.getByText("This configuration will be deleted on save."),
		).toBeInTheDocument();
		expect(screen.getByText("Undo")).toBeInTheDocument();
	});

	it("should call markForDelete when delete button is clicked", async () => {
		const user = userEvent.setup();
		const markForDelete = vi.fn();
		const drafts = new Map([
			["/repo/a", makeDraft({ apiToken: "token", databaseId: "db-id" })],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={vi.fn()}
				validate={vi.fn()}
				markForDelete={markForDelete}
			/>,
		);

		const trashButton = screen.getByRole("button", {
			name: "Delete Notion configuration",
		});
		await user.click(trashButton);
		expect(markForDelete).toHaveBeenCalledWith("/repo/a");
	});

	it("should call updateDraft when toggling a label checkbox", async () => {
		const user = userEvent.setup();
		const updateDraft = vi.fn();
		const drafts = new Map([
			[
				"/repo/a",
				makeDraft({
					apiToken: "token",
					databaseId: "db-id",
					validationStatus: "success",
					properties: [
						{ name: "Name", property_type: "title", options: [] },
						{ name: "Status", property_type: "select", options: [] },
					],
				}),
			],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={updateDraft}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		const checkbox = screen.getByRole("checkbox", {
			name: "Status (select)",
		});
		await user.click(checkbox);
		expect(updateDraft).toHaveBeenCalledWith("/repo/a", expect.any(Function));
	});

	it("should call updateDraft when selecting a Title property", async () => {
		const user = userEvent.setup();
		const updateDraft = vi.fn();
		const drafts = new Map([
			[
				"/repo/a",
				makeDraft({
					apiToken: "token",
					databaseId: "db-id",
					validationStatus: "success",
					properties: [
						{ name: "Name", property_type: "title", options: [] },
						{ name: "Summary", property_type: "rich_text", options: [] },
					],
				}),
			],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={updateDraft}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		const triggers = screen.getAllByRole("combobox");
		await user.click(triggers[0]);
		await user.click(
			screen.getByRole("option", { name: "Summary (rich_text)" }),
		);
		expect(updateDraft).toHaveBeenCalledWith("/repo/a", expect.any(Function));
	});

	it("should call updateDraft when selecting a Branch Name property", async () => {
		const user = userEvent.setup();
		const updateDraft = vi.fn();
		const drafts = new Map([
			[
				"/repo/a",
				makeDraft({
					apiToken: "token",
					databaseId: "db-id",
					validationStatus: "success",
					properties: [
						{ name: "Name", property_type: "title", options: [] },
						{ name: "ID", property_type: "number", options: [] },
					],
				}),
			],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={updateDraft}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		const triggers = screen.getAllByRole("combobox");
		await user.click(triggers[1]);
		await user.click(screen.getByRole("option", { name: "ID (number)" }));
		expect(updateDraft).toHaveBeenCalledWith("/repo/a", expect.any(Function));
	});

	it("should call updateDraft when typing a prefix", async () => {
		const user = userEvent.setup();
		const updateDraft = vi.fn();
		const drafts = new Map([
			[
				"/repo/a",
				makeDraft({
					apiToken: "token",
					databaseId: "db-id",
					validationStatus: "success",
					properties: [{ name: "Name", property_type: "title", options: [] }],
				}),
			],
		]);

		render(
			<NotionSettingsSection
				repoPaths={["/repo/a"]}
				drafts={drafts}
				updateDraft={updateDraft}
				validate={vi.fn()}
				markForDelete={vi.fn()}
			/>,
		);

		const prefixInput = screen.getByLabelText("Prefix");
		await user.type(prefixInput, "f");
		expect(updateDraft).toHaveBeenCalledWith("/repo/a", expect.any(Function));
	});
});
