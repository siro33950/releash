import SwiftUI

struct CreateWorktreeView: View {
  @Bindable var model: AppModel
  @State private var form = CreateWorktreeForm()
  @Environment(\.dismiss) private var dismiss
  @State private var source = "branch"
  @State private var issues: [Releash_Client_V1_IssueInfoDto] = []
  @State private var labels: [String] = []
  @State private var milestones: [String] = []
  @State private var allIssues: [Releash_Client_V1_IssueInfoDto] = []
  @State private var tasks: [Releash_Client_V1_NotionTaskView] = []
  @State private var branches: [Releash_Client_V1_BranchStatus] = []
  @State private var label = ""
  @State private var milestone = ""
  @State private var readFailure: String?
  @State private var issueArgs: [String] = []
  var body: some View {
    Form {
      Picker("Repository", selection: $form.repository) {
        Text("選択してください").tag("")
        ForEach(model.repositories, id: \.path) { Text($0.path).tag($0.path) }
      }
      TextField("base branch", text: $form.baseBranch)
      Picker("作成元", selection: $source) {
        Text("Branch").tag("branch")
        Text("GitHub Issue").tag("issue")
        Text("Notion task").tag("notion")
      }.pickerStyle(.segmented)
      if source == "issue" {
        HStack {
          Picker("Label", selection: $label) {
            Text("すべて").tag("")
            ForEach(labels, id: \.self) { Text($0).tag($0) }
          }
          Picker("Milestone", selection: $milestone) {
            Text("すべて").tag("")
            ForEach(milestones, id: \.self) { Text($0).tag($0) }
          }
        }
        List(issues, id: \.number) { issue in
          selectionRow(
            "#\(issue.number) \(issue.title)", id: .issue(issue.number),
            branch: issue.defaultBranchName)
        }
      } else if source == "notion" {
        List(tasks, id: \.id) { task in
          selectionRow(task.title, id: .task(task.id), branch: task.branchName)
        }
      }
      DisclosureGroup("Advanced") {
        TextField("新しいbranch名", text: $form.branch).disabled(!form.selected.isEmpty)
        List(branches, id: \.name) { item in
          selectionRow(item.name, id: .branch(item.name), branch: item.name)
        }
      }
      if let readFailure { Text(readFailure).foregroundStyle(.secondary) }
      Picker("作成後に起動する", selection: $form.launch) {
        Text("起動しない").tag("none")
        Text("Session").tag("session")
        Text("Workflow").tag("workflow")
      }
      if form.launch == "session" {
        Picker("Provider", selection: $form.provider) {
          ForEach(model.providers, id: \.value) { provider in
            Text(provider.displayName).tag(
              provider.wireName)
          }
        }
      }
      if form.launch == "workflow" {
        WorkflowLaunchFields(
          workflows: model.workflows, workflow: $form.workflow, requestText: $form.requestText)
      }
      HStack {
        Button("キャンセル") { dismiss() }
        Spacer()
        Button("作成") {
          form.submit(model: model) { dismiss() }
        }
        .disabled(!form.canSubmit)
      }
    }
    .padding(UIStyle.padding)
    .frame(minWidth: UIStyle.sidebarWidth * 2)
    .onChange(of: form.repository) { old, _ in
      unsubscribe(old)
      load()
    }
    .onChange(of: source) { _, _ in
      form.selected = WorktreeSelection()
      form.branch = ""
    }
    .onChange(of: label) { _, _ in filterIssues() }
    .onChange(of: milestone) { _, _ in filterIssues() }
    .onDisappear { unsubscribe(form.repository) }
  }
  private func selectionRow(_ title: String, id: WorktreeSelection.Key, branch: String) -> some View
  {
    Toggle(
      title,
      isOn: Binding(
        get: { form.selected.contains(id) },
        set: {
          form.selected.set(id, branch: branch, selected: $0)
        }))
  }
  private func load() {
    issues = []
    allIssues = []
    labels = []
    milestones = []
    tasks = []
    branches = []
    form.selected = WorktreeSelection()
    label = ""
    milestone = ""
    readFailure = nil
    guard !form.repository.isEmpty else { return }
    model.subscribe("available-branches", [form.repository]) { payload in
      if case .branchStatus(let value) = payload.value { branches = value.items }
    }
    model.subscribe("issues", [form.repository]) { payload in
      if case .issues(let value) = payload.value {
        labels = value.labels
        milestones = value.milestones
        allIssues = value.issues.items
        if label.isEmpty && milestone.isEmpty { issues = value.issues.items }
        if value.hasReadError { readFailure = value.readError }
      }
    }
    model.subscribe("notion-tasks", [form.repository, "100"]) { payload in
      if case .notionTasks(let value) = payload.value {
        tasks = value.page.tasks.items
        if value.hasReadError { readFailure = value.readError.message }
      }
    }
  }
  private func filterIssues() {
    if !issueArgs.isEmpty { model.unsubscribe("issues", issueArgs) }
    if label.isEmpty && milestone.isEmpty {
      issueArgs = []
      issues = allIssues
      return
    }
    issueArgs = [form.repository]
    if !label.isEmpty { issueArgs.append("label=\(label)") }
    if !milestone.isEmpty { issueArgs.append("milestone=\(milestone)") }
    model.subscribe("issues", issueArgs) { payload in
      if case .issues(let value) = payload.value {
        issues = value.issues.items
        if value.hasReadError { readFailure = value.readError }
      }
    }
  }
  private func unsubscribe(_ path: String) {
    guard !path.isEmpty else { return }
    model.unsubscribe("available-branches", [path])
    model.unsubscribe("issues", [path])
    model.unsubscribe("notion-tasks", [path, "100"])
    if !issueArgs.isEmpty {
      model.unsubscribe("issues", issueArgs)
      issueArgs = []
    }
  }
}

struct CreateWorktreeForm {
  var repository = ""
  var baseBranch = ""
  var branch = ""
  var selected = WorktreeSelection()
  var launch = "none"
  var provider = "claude"
  var workflow = ""
  var requestText = ""
  var selectedBranches: [String] {
    if selected.isEmpty {
      return branch.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? [] : [branch]
    }
    return selected.branches
  }
  var canSubmit: Bool {
    !repository.isEmpty && !selectedBranches.isEmpty && (launch != "workflow" || !workflow.isEmpty)
  }
  @MainActor func submit(model: AppModel, dismiss: () -> Void) {
    guard canSubmit, let client = model.client else { return }
    var request = Releash_Client_V1_CreateWorktreesRequest()
    request.repoPath = repository
    request.branches = selectedBranches
    if !baseBranch.isEmpty { request.baseBranch = baseBranch }
    if launch == "session" {
      var session = Releash_Client_V1_WorktreeSessionLaunch()
      session.provider = provider
      session.rows = UIStyle.terminalRows
      session.cols = UIStyle.terminalColumns
      session.requestID = UUID().uuidString
      request.launch = .session(session)
    } else if launch == "workflow" {
      var workflow = Releash_Client_V1_WorktreeWorkflowLaunch()
      workflow.name = self.workflow
      workflow.request = requestText
      request.launch = .workflow(workflow)
    }
    model.perform {
      _ = try await client.createWorktrees(request: request, headers: model.headers).result.get()
    }
    dismiss()
  }
}
