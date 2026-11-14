import SwiftUI

struct CreateWorktreeView: View {
  @Bindable var model: AppModel
  @State var form = CreateWorktreeForm()
  @Environment(\.dismiss) private var dismiss
  @State private var bases = Releash_Client_V1_WorktreeBasesSnapshot()
  @State private var branches: [Releash_Client_V1_BranchStatus] = []
  @State private var issues = Releash_Client_V1_IssuesSnapshot()
  @State private var tasks: [Releash_Client_V1_NotionTaskView] = []
  @State private var notionOptions: [Releash_Client_V1_NotionLabelOption] = []
  @State private var readFailure: String?
  @State private var optionsFailure: String?
  @State private var listSubscription: UUID?
  @State private var baseSubscription: UUID?
  @State private var optionSubscription: UUID?
  @State private var formHeight: CGFloat = 0
  @State private var formInset: CGFloat = 0
  @State private var listTop: CGFloat = 0
  private static let formSpace = "create-worktree-form"

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Text("新しいWorktree").font(.headline)
        Spacer()
      }.padding(UIStyle.padding)
      Divider()
      Form {
        Section {
          Picker("Repository", selection: $form.repository) {
            if form.repository.isEmpty { Text("選択してください").tag("") }
            ForEach(model.repositories, id: \.path) { repository in
              Text(repository.name).help(repository.path).tag(repository.path)
            }
          }
          .help(form.repository)
          .accessibilityIdentifier("repository-picker")
          .onGeometryChange(for: CGFloat.self) {
            $0.frame(in: .named(Self.formSpace)).minY
          } action: {
            formInset = $0
          }
          Picker("base branch", selection: $form.baseBranch) {
            if form.baseBranch.isEmpty { Text("今の HEAD").tag("") }
            ForEach(bases.branches, id: \.self) { Text($0).tag($0) }
          }
          .disabled(form.source == .branch)
          .accessibilityIdentifier("base-branch-picker")
        }
        Section("作成元") {
          Picker("作成元", selection: $form.source) {
            ForEach(CreateWorktreeForm.Source.allCases, id: \.self) { Text($0.rawValue).tag($0) }
          }
          .pickerStyle(.segmented).labelsHidden()
          .frame(maxWidth: .infinity)
          .accessibilityIdentifier("worktree-source")
          HStack(spacing: UIStyle.spacing) {
            TextField(text: $form.search, prompt: Text(form.source.searchPrompt)) { Text("検索") }
              .textFieldStyle(.roundedBorder).labelsHidden()
              .accessibilityIdentifier("worktree-source-search")
            filters
          }
          ScrollView {
            LazyVStack(alignment: .leading, spacing: 0) {
              if let failure = readFailure ?? (form.source == .notion ? optionsFailure : nil) {
                Text(failure).foregroundStyle(.secondary).padding(.vertical, UIStyle.spacing)
                  .accessibilityIdentifier("worktree-source-failure")
              }
              rows
            }
          }
          .frame(height: max(0, formHeight - listTop - formInset))
          .onGeometryChange(for: CGFloat.self) {
            $0.frame(in: .named(Self.formSpace)).minY
          } action: {
            listTop = $0
          }
        }
      }
      .formStyle(.grouped).scrollDisabled(true)
      .coordinateSpace(.named(Self.formSpace))
      .onGeometryChange(for: CGFloat.self) {
        $0.size.height
      } action: {
        formHeight = $0
      }
      Divider()
      HStack {
        Spacer()
        Button("キャンセル") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("作成") {
          form.submit(model: model) { dismiss() }
        }
        .keyboardShortcut(.defaultAction)
        .disabled(!form.canSubmit)
      }.padding(UIStyle.padding)
    }
    .frame(width: UIStyle.worktreeDialogWidth, height: UIStyle.worktreeDialogHeight)
    .onAppear {
      guard form.repository.isEmpty else { return }
      form.repository =
        model.repositories.first { repository in
          repository.branches.items.contains { $0.worktreePath == model.selectedWorktree }
        }?.path ?? model.repositories.first?.path ?? ""
    }
    .onChange(of: form.repository, initial: true) { _, repository in
      bases = .init()
      branches = []
      issues = .init()
      tasks = []
      readFailure = nil
      unsubscribe(&baseSubscription)
      guard !repository.isEmpty else { return }
      baseSubscription = model.subscribe("worktree-bases", [repository]) { payload in
        guard case .worktreeBases(let value) = payload.value else { return }
        bases = value
        if !value.branches.contains(form.baseBranch) { form.baseBranch = value.defaultBranch }
      }
    }
    .onChange(of: form.source) { _, _ in readFailure = nil }
    .onChange(of: form.notionOptionsRepository, initial: true) { _, repository in
      notionOptions = []
      optionsFailure = nil
      unsubscribe(&optionSubscription)
      guard let repository else { return }
      optionSubscription = model.subscribe("notion-label-options", [repository]) { payload in
        guard case .notionLabelOptions(let value) = payload.value else { return }
        notionOptions = value.options
        optionsFailure = value.hasReadError ? value.readError.message : nil
        form.notionFilters = form.notionFilters.filter { name, selected in
          notionOptions.contains { option in
            option.propertyName == name && option.choices.contains { $0.value == selected }
          }
        }
      }
    }
    .task(id: form.list) { [list = form.list, searching = !form.searchTerm.isEmpty] in
      unsubscribe(&listSubscription)
      guard let list else { return }
      if searching { try? await Task.sleep(for: .milliseconds(250)) }
      guard !Task.isCancelled else { return }
      listSubscription = model.subscribe(list.target, list.arguments) { receive($0) }
    }
    .onDisappear {
      unsubscribe(&listSubscription)
      unsubscribe(&baseSubscription)
      unsubscribe(&optionSubscription)
    }
  }

  @ViewBuilder private var filters: some View {
    switch form.source {
    case .branch: EmptyView()
    case .issue:
      filter("Label", selection: $form.label, choices: issues.labels.map { ($0, $0) })
      filter("Milestone", selection: $form.milestone, choices: issues.milestones.map { ($0, $0) })
    case .notion:
      ForEach(notionOptions, id: \.propertyName) { option in
        filter(
          option.propertyName, selection: $form[notionFilter: option.propertyName],
          choices: option.choices.map { ($0.label, $0.value) })
      }
    }
  }

  @ViewBuilder private func filter(
    _ title: String, selection: Binding<String>, choices: [(String, String)]
  ) -> some View {
    Text(title)
    Picker(title, selection: selection) {
      Text("すべて").tag("")
      ForEach(choices, id: \.1) { Text($0.0).tag($0.1) }
    }
    .labelsHidden().fixedSize()
    .accessibilityIdentifier("worktree-source-filter:\(title)")
  }

  @ViewBuilder private var rows: some View {
    switch form.source {
    case .branch:
      ForEach(branches, id: \.name) { branch in
        row(.branch(branch.name), branch: branch.name) {
          Text(branch.name).font(.body.monospaced())
        }
      }
    case .issue:
      ForEach(issues.issues.items, id: \.number) { issue in
        row(.issue(issue.number), branch: issue.defaultBranchName) {
          HStack(spacing: UIStyle.spacing) {
            Text(verbatim: "#\(issue.number)").font(.body.monospaced()).foregroundStyle(.secondary)
            Text(issue.title)
          }
        }
      }
    case .notion:
      ForEach(tasks, id: \.id) { task in
        row(.task(task.id), branch: task.branchName) { Text(task.title) }
      }
    }
  }

  private func row<Label: View>(
    _ key: WorktreeSelection.Key, branch: String, @ViewBuilder label: () -> Label
  ) -> some View {
    VStack(spacing: 0) {
      Toggle(
        isOn: Binding(
          get: { form.selected.contains(key) },
          set: { form.selected.set(key, branch: branch, selected: $0) }), label: label
      )
      .toggleStyle(.checkbox)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.vertical, UIStyle.spacing)
      .accessibilityIdentifier("worktree-source-item:\(key.identifier)")
      Divider()
    }
  }

  private func receive(_ payload: Releash_Client_V1_StatePayload) {
    switch payload.value {
    case .branchStatus(let value):
      branches = value.items
      readFailure = nil
    case .issues(let value):
      issues = value
      readFailure = value.hasReadError ? value.readError : nil
      if !value.labels.contains(form.label) { form.label = "" }
      if !value.milestones.contains(form.milestone) { form.milestone = "" }
    case .notionTasks(let value):
      tasks = value.page.tasks.items
      readFailure = value.hasReadError ? value.readError.message : nil
    default: break
    }
  }

  private func unsubscribe(_ subscription: inout UUID?) {
    if let subscription { model.unsubscribe(subscription) }
    subscription = nil
  }
}

struct CreateWorktreeForm {
  enum Source: String, CaseIterable {
    case branch = "Branch"
    case issue = "GitHub Issue"
    case notion = "Notion task"

    var searchPrompt: String {
      switch self {
      case .branch: "ブランチを検索"
      case .issue: "Issue を検索"
      case .notion: "task を検索"
      }
    }
  }
  struct Subscription: Equatable {
    var target: String
    var arguments: [String]
  }

  var repository = "" {
    didSet {
      guard repository != oldValue else { return }
      baseBranch = ""
      label = ""
      milestone = ""
      notionFilters = [:]
      selected = WorktreeSelection()
      readsNotionOptions = source == .notion
    }
  }
  var baseBranch = ""
  var source = Source.branch {
    didSet {
      if source != oldValue { selected = WorktreeSelection() }
      if source == .notion { readsNotionOptions = true }
    }
  }
  var search = ""
  var label = ""
  var milestone = ""
  var notionFilters: [String: String] = [:]
  var selected = WorktreeSelection()

  /// Notion のプロパティごとの絞り込み。空は絞り込まないことを表す
  subscript(notionFilter property: String) -> String {
    get { notionFilters[property] ?? "" }
    set { notionFilters[property] = newValue.isEmpty ? nil : newValue }
  }
  var searchTerm: String { search.trimmingCharacters(in: .whitespacesAndNewlines) }
  var canSubmit: Bool { !repository.isEmpty && !selected.isEmpty }
  private var readsNotionOptions = false
  /// Notion の絞り込みの選択肢を読む Repository。Notion task を選んでから、Repository を変えるまで読む
  var notionOptionsRepository: String? {
    readsNotionOptions && !repository.isEmpty ? repository : nil
  }
  /// 作成元の一覧の購読。検索と絞り込みは引数でサーバへ渡す
  var list: Subscription? {
    guard !repository.isEmpty else { return nil }
    let term = searchTerm
    switch source {
    case .branch:
      return Subscription(
        target: "available-branches",
        arguments: [repository] + (term.isEmpty ? [] : ["search=\(term)"]))
    case .issue:
      return Subscription(
        target: "issues",
        arguments: [repository] + (label.isEmpty ? [] : ["label=\(label)"])
          + (milestone.isEmpty ? [] : ["milestone=\(milestone)"])
          + (term.isEmpty ? [] : ["search=\(term)"]))
    case .notion:
      let encoder = JSONEncoder()
      encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
      let labels = try? encoder.encode(notionFilters.mapValues { [$0] })
      return Subscription(
        target: "notion-tasks",
        arguments: [repository, "100"] + (term.isEmpty ? [] : ["title=\(term)"])
          + (notionFilters.isEmpty
            ? [] : labels.map { ["labels=" + String(decoding: $0, as: UTF8.self)] } ?? []))
    }
  }

  @MainActor func submit(model: AppModel, dismiss: () -> Void) {
    guard canSubmit, let client = model.client else { return }
    var request = Releash_Client_V1_CreateWorktreesRequest()
    request.repoPath = repository
    request.branches = selected.branches
    if !baseBranch.isEmpty { request.baseBranch = baseBranch }
    model.perform {
      _ = try await client.createWorktrees(request: request, headers: model.headers).result.get()
    }
    dismiss()
  }
}
