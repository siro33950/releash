import Connect
import SwiftProtobuf
import SwiftUI

private enum WorkflowSettingsRoute: Hashable {
  case workflow(String)
  case facet(String, String)
}

struct WorkflowsSettingsView: View {
  var model: AppModel
  @State var state = WorkflowsSettingsState()
  @State private var mode = "定義"
  @State private var kind = "policy"

  var body: some View {
    NavigationStack {
      VStack {
        Picker("表示", selection: $mode) {
          Text("定義").tag("定義")
          Text("Facet").tag("Facet")
        }
        .pickerStyle(.segmented)
        .accessibilityIdentifier("settings-workflows-mode")
        if mode == "Facet" {
          Picker("種類", selection: $kind) {
            ForEach(["policy", "knowledge", "instruction"], id: \.self) { Text($0).tag($0) }
          }
          .pickerStyle(.segmented)
          .accessibilityIdentifier("settings-facet-kind")
        }
        List {
          if mode == "定義" {
            ForEach(state.workflows.items, id: \.name) { workflow in
              NavigationLink(value: WorkflowSettingsRoute.workflow(workflow.name)) {
                VStack(alignment: .leading) {
                  Text(workflow.name).font(.headline)
                  Text(workflow.description_p)
                  HStack {
                    if workflow.builtin { Text("builtin") }
                    if workflow.isRunning { Text("実行中") }
                    DiagnosticCounts(
                      summary: state.diagnostics.workflowSummaries.entries[workflow.name])
                  }.foregroundStyle(.secondary)
                  if workflow.hasReadError { Text(workflow.readError) }
                }
              }
              .accessibilityIdentifier("settings-workflow-\(workflow.name)")
            }
          } else {
            ForEach(state.facets.items, id: \.key) { facet in
              NavigationLink(value: WorkflowSettingsRoute.facet(facet.kind, facet.key)) {
                VStack(alignment: .leading) {
                  Text(facet.key).font(.headline)
                  Text(facet.description_p)
                  HStack {
                    if facet.builtin { Text("builtin") }
                    DiagnosticCounts(
                      summary: state.diagnostics.facetSummaries.entries[
                        "\(facet.kind)/\(facet.key)"])
                  }.foregroundStyle(.secondary)
                }
              }
              .accessibilityIdentifier("settings-facet-\(facet.kind)-\(facet.key)")
            }
          }
        }
        Button("定義のフォルダを外部エディタで開く") {
          Task { await state.openDirectory(model) }
        }
        .disabled(!model.connected || state.saving.contains(.directory))
        .accessibilityIdentifier("settings-open-workflows-directory")
        HStack {
          Text("error: \(state.diagnostics.total.errorCount)")
          Text("info: \(state.diagnostics.total.infoCount)")
        }
        .foregroundStyle(.secondary)
        WorkflowSettingsFailure(failure: state.failure)
      }
      .padding()
      .navigationDestination(for: WorkflowSettingsRoute.self) { route in
        switch route {
        case .workflow(let name): WorkflowSettingsDetail(model: model, name: name)
        case .facet(let kind, let key): FacetSettingsDetail(model: model, kind: kind, key: key)
        }
      }
    }
    .onAppear {
      state.subscribe(model)
      state.selectFacetKind(kind, model)
    }
    .onDisappear { state.unsubscribe(model) }
    .onChange(of: kind) { _, value in state.selectFacetKind(value, model) }
  }
}

private struct WorkflowSettingsDetail: View {
  var model: AppModel
  let name: String
  @Environment(\.dismiss) private var dismiss
  @State private var state = WorkflowsSettingsState()

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Button("戻る") { dismiss() }
        Text(name).font(.title2)
        Spacer()
        if state.workflow?.hasValue == true && state.workflow?.value.builtin == false {
          Button("外部エディタで開く") { Task { await state.openWorkflow(name, model) } }
            .disabled(!model.connected || state.saving.contains(.workflow))
            .accessibilityIdentifier("settings-open-workflow-in-editor")
        }
      }
      .padding()
      Form {
        if let workflow = state.workflow, workflow.hasValue {
          Section {
            Text(workflow.value.description_p)
          }
        }
        WorkflowDiagnostics(items: state.diagnostics.items.items.filter { $0.workflowName == name })
        if let workflow = state.workflow, workflow.hasValue {
          Section("Node") {
            ForEach(workflow.value.nodes.items, id: \.name) { node in
              WorkflowNodeRow(node: node)
            }
          }
        }
        WorkflowSettingsFailure(failure: state.failure)
      }
      .formStyle(.grouped)
    }
    .navigationBarBackButtonHidden(true)
    .onAppear {
      state.subscribe(model)
      state.selectWorkflow(name, model)
    }
    .onDisappear { state.unsubscribe(model) }
  }
}

private struct FacetSettingsDetail: View {
  var model: AppModel
  let kind: String
  let key: String
  @Environment(\.dismiss) private var dismiss
  @State private var state = WorkflowsSettingsState()

  var body: some View {
    VStack(spacing: 0) {
      HStack {
        Button("戻る") { dismiss() }
        Text("\(kind) / \(key)").font(.title2)
        Spacer()
        if state.facets.items.first(where: { $0.key == key && $0.kind == kind })?.builtin == false {
          Button("外部エディタで開く") { Task { await state.openFacet(kind, key, model) } }
            .disabled(!model.connected || state.saving.contains(.facet))
            .accessibilityIdentifier("settings-open-facet-in-editor")
        }
      }
      .padding()
      Form {
        Section {
          LabeledContent("key", value: key)
          LabeledContent("種類", value: kind)
          if let facet = state.facets.items.first(where: { $0.key == key && $0.kind == kind }) {
            Text(facet.description_p)
          }
        }
        WorkflowDiagnostics(
          items: state.diagnostics.items.items.filter { $0.facetKind == kind && $0.facetKey == key }
        )
        Section("参照元") {
          ForEach(
            Array(
              (state.diagnostics.facetUsage.entries["\(kind)/\(key)"]?.items ?? []).enumerated()),
            id: \.offset
          ) { _, usage in
            NavigationLink(value: WorkflowSettingsRoute.workflow(usage.workflowName)) {
              Text("\(usage.workflowName) / \(usage.nodeName) / \(usage.slot)")
            }
            .accessibilityIdentifier("settings-facet-usage-\(usage.workflowName)-\(usage.nodeName)")
          }
        }
        WorkflowSettingsFailure(failure: state.failure)
      }
      .formStyle(.grouped)
    }
    .navigationBarBackButtonHidden(true)
    .onAppear {
      state.subscribe(model)
      state.selectFacetKind(kind, model)
    }
    .onDisappear { state.unsubscribe(model) }
  }
}

private struct WorkflowNodeRow: View {
  let node: Releash_Client_V1_NodeDefinitionDto
  @State private var expanded = false

  var body: some View {
    DisclosureGroup(isExpanded: $expanded) {
      if node.hasCommand { LabeledContent("command", value: node.command) }
      if node.hasSession {
        LabeledContent("provider", value: String(describing: node.session.provider.value))
        if node.session.hasModel { LabeledContent("model", value: node.session.model) }
        if node.session.hasPermission {
          LabeledContent("permission", value: node.session.permission)
        }
        if node.session.facets.hasPolicy { facetLink("policy", node.session.facets.policy) }
        ForEach(node.session.facets.knowledge.items, id: \.self) { facetLink("knowledge", $0) }
        if node.session.facets.hasInstruction {
          facetLink("instruction", node.session.facets.instruction)
        }
      }
      if node.hasFanout {
        ForEach(node.fanout.children.items, id: \.name) { LabeledContent("子Node", value: $0.name) }
        if node.fanout.hasItems {
          LabeledContent("items", value: node.fanout.items.textFormatString())
        }
      }
      if node.hasSequence {
        if node.sequence.hasEntry { LabeledContent("entry", value: node.sequence.entry) }
        ForEach(node.sequence.children.items, id: \.name) {
          LabeledContent("子Node", value: $0.name)
        }
      }
      if node.hasArtifact { LabeledContent("artifact", value: node.artifact) }
      ForEach(node.input.items, id: \.name) { input in
        LabeledContent("input", value: input.name)
        if input.hasContract { LabeledContent("contract", value: input.contract) }
      }
      if node.hasCompletion {
        if node.completion.hasRequire {
          LabeledContent("require", value: String(describing: node.completion.require.value))
        }
        if node.completion.hasDelegate {
          LabeledContent("delegate", value: node.completion.delegate.textFormatString())
        }
      }
      if node.hasWorktree {
        LabeledContent("worktree", value: String(describing: node.worktree.value))
      }
    } label: {
      Text("\(node.name) / \(String(describing: node.kind.value))")
        .accessibilityIdentifier("settings-workflow-node-\(node.name)")
        .accessibilityAddTraits(.isButton)
        .accessibilityAction { expanded.toggle() }
    }
  }

  private func facetLink(_ kind: String, _ key: String) -> some View {
    NavigationLink(value: WorkflowSettingsRoute.facet(kind, key)) { Text("\(kind): \(key)") }
  }
}

private struct DiagnosticCounts: View {
  let summary: Releash_Client_V1_DiagnosticSummary?
  var body: some View {
    if let summary {
      if summary.errorCount > 0 { Text("error: \(summary.errorCount)") }
    }
  }
}

private struct WorkflowDiagnostics: View {
  let items: [Releash_Client_V1_DiagnosticItem]
  var body: some View {
    Section("診断") {
      ForEach(Array(items.enumerated()), id: \.offset) { _, item in
        VStack(alignment: .leading) {
          Text(
            "\(item.code) / \(String(describing: item.severity.value)) / \(stage(item.stage.value))"
          )
          if item.hasSpan {
            Text(
              "\(item.span.source):\(item.span.startLine):\(item.span.startCol)–\(item.span.endLine):\(item.span.endCol)"
            )
          }
          Text(item.message)
        }
        .accessibilityIdentifier("settings-diagnostic-\(item.code)")
      }
    }
  }

  private func stage(_ value: Releash_Client_V1_DiagnosticStage.Value) -> String {
    switch value {
    case .parseShape: "parse_shape"
    case .resolve: "resolve"
    case .typecheck: "typecheck"
    case .controlFlow: "control_flow"
    case .UNRECOGNIZED(let rawValue): String(rawValue)
    }
  }
}

private struct WorkflowSettingsFailure: View {
  let failure: String?
  var body: some View {
    if let failure {
      Text(failure).foregroundStyle(.secondary)
        .accessibilityIdentifier("settings-workflows-failure")
    }
  }
}

@Observable @MainActor final class WorkflowsSettingsState: SettingsSaving {
  enum Field { case workflow, facet, directory }
  var workflows = Releash_Client_V1_ListWorkflowSummaryDto()
  var workflow: Releash_Client_V1_NullableWorkflowDto?
  var facets = Releash_Client_V1_ListFacetSummaryDto()
  var diagnostics = Releash_Client_V1_DiagnosticReport()
  var failure: String?
  var saving: Set<Field> = []
  @ObservationIgnored private var subscriptions: [UUID] = []
  @ObservationIgnored private var workflowSubscription: UUID?
  @ObservationIgnored private var facetSubscription: UUID?

  func subscribe(_ model: AppModel) {
    guard subscriptions.isEmpty else { return }
    subscriptions = [
      model.subscribe("workflows") { [weak self] payload in
        if case .workflows(let value) = payload.value { self?.workflows = value }
      },
      model.subscribe("diagnostics") { [weak self] payload in
        if case .diagnostics(let value) = payload.value { self?.diagnostics = value }
      },
    ]
  }

  func selectWorkflow(_ name: String, _ model: AppModel) {
    if let id = workflowSubscription { model.unsubscribe(id) }
    workflow = nil
    workflowSubscription = model.subscribe("workflow", [name]) { [weak self] payload in
      if case .workflow(let value) = payload.value { self?.workflow = value }
    }
  }

  func selectFacetKind(_ kind: String, _ model: AppModel) {
    if let id = facetSubscription { model.unsubscribe(id) }
    facets = .init()
    facetSubscription = model.subscribe("facets", [kind]) { [weak self] payload in
      if case .facets(let value) = payload.value { self?.facets = value }
    }
  }

  func unsubscribe(_ model: AppModel) {
    for id in subscriptions { model.unsubscribe(id) }
    if let id = workflowSubscription { model.unsubscribe(id) }
    if let id = facetSubscription { model.unsubscribe(id) }
    subscriptions = []
    workflowSubscription = nil
    facetSubscription = nil
  }

  func openWorkflow(_ name: String, _ model: AppModel) async {
    await saveSetting(.workflow, model) { client, headers in
      var request = Releash_Client_V1_OpenWorkflowInEditorRequest()
      request.name = name
      _ = try await client.openWorkflowInEditor(request: request, headers: headers).result.get()
    }
  }

  func openFacet(_ kind: String, _ key: String, _ model: AppModel) async {
    await saveSetting(.facet, model) { client, headers in
      var request = Releash_Client_V1_OpenFacetInEditorRequest()
      request.kind = kind
      request.key = key
      _ = try await client.openFacetInEditor(request: request, headers: headers).result.get()
    }
  }

  func openDirectory(_ model: AppModel) async {
    await saveSetting(.directory, model) { client, headers in
      _ = try await client.openWorkflowDirectoryInEditor(request: .init(), headers: headers).result
        .get()
    }
  }
}
