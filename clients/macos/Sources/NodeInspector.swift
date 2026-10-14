import SwiftUI

struct NodeInspector: View {
  @Bindable var model: AppModel
  let path: String
  let nodeID: String
  @State private var detail: Releash_Client_V1_WorkspaceNodeDetailDto?
  @State private var output: Releash_Client_V1_WorkflowOutputView?
  @State private var detailSubscription: UUID?
  @State private var artifactSubscription: UUID?
  @State private var artifactNodeID: String?
  @State private var json = false

  var body: some View {
    VStack(spacing: 0) {
      if let detail {
        HStack(spacing: UIStyle.spacing) {
          Image(systemName: "circle.fill").font(.system(size: UIStyle.statusSize))
            .foregroundStyle(UIStyle.statusColor(String(describing: detail.classification.value)))
          Text(detail.title).fontWeight(.semibold)
          Text(detail.content.isCommand ? "Command" : "Session").font(.caption2).foregroundStyle(.secondary)
            .padding(.horizontal, UIStyle.rowSpacing)
            .background(Color(nsColor: .quaternarySystemFill), in: RoundedRectangle(cornerRadius: 3))
          Text(detail.model).font(.caption).foregroundStyle(.secondary)
          Spacer()
          if detail.capabilities.canRetry {
            Button("Retry") { model.retryNode(path: path, id: detail.id) }
          }
          if detail.canAbort {
            Button("Abort") { if detail.hasExecutionID { model.abortExecution(id: detail.executionID) } }
          }
        }.padding(UIStyle.spacing)
        if detail.hasErrorReason {
          Text(detail.errorReason).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, UIStyle.spacing)
        }
        Divider()
        if case .submitted(let artifact) = output?.variant {
          HStack(spacing: UIStyle.spacing) {
            Text("◆").foregroundStyle(Color.accentColor)
            Text(artifact.hasContract ? artifact.contract : "Artifact").font(.system(.body, design: .monospaced)).fontWeight(.medium)
            Spacer()
            Button(json ? "表を表示" : "JSONを表示") { json.toggle() }.accessibilityIdentifier("artifact-json")
          }.padding(UIStyle.spacing)
          Divider()
          GeometryReader { geometry in
            ScrollView([.horizontal, .vertical]) {
              if json {
                Text(artifact.structuredOutput.displayJSON).font(.system(.body, design: .monospaced)).textSelection(.enabled)
                  .frame(maxWidth: .infinity, alignment: .leading).padding(UIStyle.spacing)
              } else {
                Grid(alignment: .leading, horizontalSpacing: UIStyle.padding, verticalSpacing: 0) {
                  GridRow {
                    Text("FIELD").frame(minWidth: 150, alignment: .leading)
                    Text("VALUE").frame(maxWidth: .infinity, alignment: .leading)
                  }.font(.caption).foregroundStyle(.secondary).padding(.vertical, UIStyle.rowSpacing)
                  Divider().gridCellColumns(2).gridCellUnsizedAxes([.horizontal, .vertical])
                  ForEach(artifact.structuredOutput.displayFields) { field in
                    GridRow {
                      Text(field.name).padding(.leading, CGFloat(field.depth) * UIStyle.padding)
                      Text(field.value).textSelection(.enabled)
                    }.font(.system(.caption, design: .monospaced)).padding(.vertical, UIStyle.rowSpacing)
                    Divider().gridCellColumns(2).gridCellUnsizedAxes([.horizontal, .vertical])
                  }
                }.padding(.horizontal, UIStyle.spacing).frame(minWidth: geometry.size.width, alignment: .leading)
              }
            }.defaultScrollAnchor(.topLeading, for: .alignment)
          }
        } else {
          Text("Artifactはありません").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
      } else { Text("Nodeの詳細を待っています").foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity) }
    }
    .background(Color(nsColor: .textBackgroundColor))
    .onAppear {
      detailSubscription = model.subscribe("node-detail", [path, nodeID]) { payload in
        guard case .nodeDetail(let value) = payload.value else { return }
        detail = value.hasValue ? value.value : nil
        let id = detail.flatMap { $0.hasNodeExecutionID ? $0.nodeExecutionID : nil }
        guard id != artifactNodeID else { return }
        if let artifactSubscription { model.unsubscribe(artifactSubscription) }
        artifactSubscription = nil; artifactNodeID = id; output = nil
        if let id {
          artifactSubscription = model.subscribe("node-artifact", [id]) { payload in
            if case .workflowOutput(let value) = payload.value { output = value.hasValue ? value.value : nil }
          }
        }
      }
    }
    .onDisappear {
      if let detailSubscription { model.unsubscribe(detailSubscription) }
      if let artifactSubscription { model.unsubscribe(artifactSubscription) }
      detailSubscription = nil; artifactSubscription = nil; artifactNodeID = nil
    }
  }
}

extension Releash_Client_V1_WorkspaceNodeContentDto {
  var isCommand: Bool { if case .command = variant { return true }; return false }
}

struct ArtifactDisplayField: Identifiable {
  let id: String
  let name: String
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
      return "{" + v.entries.keys.sorted().map { key in
        var name = Self(); name.stringValue.value = key
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
  private func fields(name: String, id: String, depth: Int) -> [ArtifactDisplayField] {
    switch variant {
    case .objectValue(let v):
      return [ArtifactDisplayField(id: id, name: name, value: v.entries.isEmpty ? "{}" : "", depth: depth)] + v.entries.keys.sorted().flatMap { key in
        v.entries[key]!.fields(name: key, id: id + "/" + Self.pointerElement(key), depth: depth + 1)
      }
    case .listValue(let v):
      return [ArtifactDisplayField(id: id, name: name, value: v.items.isEmpty ? "[]" : "", depth: depth)] + v.items.enumerated().flatMap { index, value in
        value.fields(name: "[\(index)]", id: id + "/\(index)", depth: depth + 1)
      }
    default:
      let value: String
      if case .stringValue(let v) = variant { value = v.value } else { value = displayJSON }
      return [ArtifactDisplayField(id: id, name: name, value: value, depth: depth)]
    }
  }
}
