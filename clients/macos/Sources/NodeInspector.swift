import SwiftUI

struct NodeInspector: View {
  @Bindable var model: AppModel
  let path: String
  let nodeID: String
  /// 実行木の行に出している名前。行が無くなった Node では詳細の名前を出す
  var name: String?
  @State private var detail: Releash_Client_V1_WorkspaceNodeDetailDto?
  @State private var output: Releash_Client_V1_WorkflowOutputView?
  @State private var detailSubscription: UUID?
  @State private var artifactSubscription: UUID?
  @State private var artifactNodeID: String?
  @State private var json = false

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      if let detail {
        HStack(spacing: UIStyle.spacing) {
          Image(detail.content.kind.icon).resizable()
            .frame(width: UIStyle.statusSize, height: UIStyle.statusSize)
            .foregroundStyle(UIStyle.statusColor(String(describing: detail.classification.value)))
          Text(name ?? detail.title).fontWeight(.semibold).lineLimit(1)
            .accessibilityIdentifier("node-detail-title")
          Text(detail.metricsLabel).font(.caption.monospaced()).foregroundStyle(.secondary)
            .lineLimit(1).layoutPriority(-1)
          Spacer(minLength: 0)
          Button("Retry") { model.retryNode(path: path, id: detail.id) }
            .disabled(!detail.capabilities.canRetry)
          Button("Abort") {
            if detail.hasExecutionID { model.abortExecution(id: detail.executionID) }
          }
          .disabled(!detail.canAbort)
        }.padding(UIStyle.spacing)
        if detail.hasErrorReason {
          HStack(spacing: UIStyle.rowSpacing) {
            CardIcon(image: Image("circle-alert")).foregroundStyle(UIStyle.failure)
            Text(detail.errorReason).lineLimit(1).textSelection(.enabled)
              .accessibilityIdentifier("node-detail-failure")
          }
          .help(detail.errorReason)
          .padding([.horizontal, .bottom], UIStyle.spacing)
        }
        if detail.hasWorktree {
          let worktree = detail.worktree.branch + " · " + detail.worktree.path
          HStack(spacing: UIStyle.rowSpacing) {
            CardIcon(image: Image("git-branch"))
            Text(worktree).font(.caption).lineLimit(1).textSelection(.enabled)
              .accessibilityIdentifier("node-detail-worktree")
          }
          .foregroundStyle(.secondary).help(worktree)
          .padding([.horizontal, .bottom], UIStyle.spacing)
        }
        Divider()
        if case .submitted(let artifact) = output?.variant {
          let fields = artifact.structuredOutput.displayFields
          HStack(spacing: UIStyle.spacing) {
            Text(verbatim: "◆").foregroundStyle(Color.accentColor)
            Text(artifact.hasContract ? artifact.contract : "Artifact")
              .font(.body.monospaced()).fontWeight(.medium).lineLimit(1)
            Text(fields.count == 1 ? "1 field" : "\(fields.count) fields")
              .font(.caption).foregroundStyle(.secondary)
            Spacer()
            Button(json ? "Show Table" : "Show JSON") { json.toggle() }
              .accessibilityIdentifier("artifact-json")
          }.padding(UIStyle.spacing)
          Divider()
          GeometryReader { geometry in
            ScrollView([.horizontal, .vertical]) {
              if json {
                Text(artifact.structuredOutput.displayJSON).font(.body.monospaced())
                  .textSelection(.enabled)
                  .frame(maxWidth: .infinity, alignment: .leading).padding(UIStyle.spacing)
              } else {
                Grid(alignment: .leading, horizontalSpacing: UIStyle.padding, verticalSpacing: 0) {
                  GridRow {
                    Text("FIELD")
                      .frame(minWidth: UIStyle.artifactFieldMinimumWidth, alignment: .leading)
                    Text("TYPE")
                    Text("VALUE").frame(maxWidth: .infinity, alignment: .leading)
                  }.font(.caption).foregroundStyle(.secondary).padding(
                    .vertical, UIStyle.rowSpacing)
                  Divider().gridCellColumns(3).gridCellUnsizedAxes([.horizontal, .vertical])
                  ForEach(fields) { field in
                    GridRow {
                      Text(field.name).padding(.leading, CGFloat(field.depth) * UIStyle.padding)
                      Text(field.type).foregroundStyle(.secondary)
                      Text(field.value).textSelection(.enabled)
                    }.font(.caption.monospaced()).padding(.vertical, UIStyle.rowSpacing)
                    Divider().gridCellColumns(3).gridCellUnsizedAxes([.horizontal, .vertical])
                  }
                }.padding(.horizontal, UIStyle.spacing).frame(
                  minWidth: geometry.size.width, alignment: .leading)
              }
            }.defaultScrollAnchor(.topLeading, for: .alignment)
          }
        } else {
          Text("No Artifact").foregroundStyle(.secondary).padding(UIStyle.spacing)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
      } else {
        Text("Waiting for the Node detail").foregroundStyle(.secondary).frame(
          maxWidth: .infinity, maxHeight: .infinity)
      }
    }
    .onAppear {
      if let detailSubscription { model.unsubscribe(detailSubscription) }
      detailSubscription = model.subscribe("node-detail", [path, nodeID]) { payload in
        guard case .nodeDetail(let value) = payload.value else { return }
        detail = value.hasValue ? value.value : nil
        let id = detail.flatMap { $0.hasNodeExecutionID ? $0.nodeExecutionID : nil }
        guard id != artifactNodeID else { return }
        if let artifactSubscription { model.unsubscribe(artifactSubscription) }
        artifactSubscription = nil
        artifactNodeID = id
        output = nil
        if let id {
          artifactSubscription = model.subscribe("node-artifact", [id]) { payload in
            if case .workflowOutput(let value) = payload.value {
              output = value.hasValue ? value.value : nil
            }
          }
        }
      }
    }
    .onDisappear {
      if let detailSubscription { model.unsubscribe(detailSubscription) }
      if let artifactSubscription { model.unsubscribe(artifactSubscription) }
      detailSubscription = nil
      artifactSubscription = nil
      artifactNodeID = nil
    }
  }
}

extension Releash_Client_V1_WorkspaceNodeContentDto {
  var kind: UIStyle.NodeKind {
    switch variant {
    case .command: .command
    case .sequence: .sequence
    case .fanout: .fanout
    case .session, nil: .session
    }
  }
}

extension Releash_Client_V1_WorkspaceNodeDetailDto {
  var metricsLabel: String {
    let model = content.kind.modelLabel ?? (hasModel && !self.model.isEmpty ? self.model : nil)
    return ((model.map { [$0] } ?? []) + ["Elapsed", "Active", "Token", "Cost"].map { $0 + " —" })
      .joined(separator: " · ")
  }
}

struct ArtifactDisplayField: Identifiable {
  let id: String
  let name: String
  let type: String
  let value: String
  let depth: Int
}

extension Releash_Client_V1_WorkflowValue {
  var displayJSON: String {
    switch variant {
    case .nullValue, nil: return "null"
    case .booleanValue(let v): return v.value ? "true" : "false"
    case .signedInteger(let v): return String(v.value)
    case .unsignedInteger(let v): return String(v.value)
    case .numberValue(let v): return String(v.value)
    case .stringValue(let v):
      let data = try! JSONSerialization.data(withJSONObject: v.value, options: [.fragmentsAllowed])
      return String(decoding: data, as: UTF8.self)
    case .listValue(let v): return "[" + v.items.map(\.displayJSON).joined(separator: ", ") + "]"
    case .objectValue(let v):
      return "{"
        + v.entries.keys.sorted().map { key in
          var name = Self()
          name.stringValue.value = key
          return name.displayJSON + ": " + v.entries[key]!.displayJSON
        }.joined(separator: ", ") + "}"
    }
  }
  var displayFields: [ArtifactDisplayField] {
    if case .objectValue(let object) = variant, !object.entries.isEmpty {
      return object.entries.keys.sorted().flatMap { key in
        object.entries[key]!.fields(name: key, id: "/" + Self.pointerElement(key), depth: 0)
      }
    }
    return fields(name: "$", id: "$", depth: 0)
  }
  private static func pointerElement(_ key: String) -> String {
    key.replacingOccurrences(of: "~", with: "~0").replacingOccurrences(of: "/", with: "~1")
  }
  var typeName: String {
    switch variant {
    case .nullValue, nil: "null"
    case .booleanValue: "boolean"
    case .signedInteger, .unsignedInteger: "integer"
    case .numberValue: "number"
    case .stringValue: "string"
    case .listValue(let v): "array[\(v.items.count)]"
    case .objectValue: "object"
    }
  }
  private func fields(name: String, id: String, depth: Int) -> [ArtifactDisplayField] {
    switch variant {
    case .objectValue(let v):
      return [
        ArtifactDisplayField(
          id: id, name: name, type: typeName, value: v.entries.isEmpty ? "{}" : "", depth: depth)
      ]
        + v.entries.keys.sorted().flatMap { key in
          v.entries[key]!.fields(
            name: key, id: id + "/" + Self.pointerElement(key), depth: depth + 1)
        }
    case .listValue(let v):
      return [
        ArtifactDisplayField(
          id: id, name: name, type: typeName, value: v.items.isEmpty ? "[]" : "", depth: depth)
      ]
        + v.items.enumerated().flatMap { index, value in
          value.fields(name: "[\(index)]", id: id + "/\(index)", depth: depth + 1)
        }
    default:
      let value: String
      if case .stringValue(let v) = variant { value = v.value } else { value = displayJSON }
      return [ArtifactDisplayField(id: id, name: name, type: typeName, value: value, depth: depth)]
    }
  }
}
