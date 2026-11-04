import AppKit
import ApplicationServices
import Connect
import Foundation
import SwiftProtobuf
import SwiftUI
import Synchronization
import XCTest

@testable import Releash

final class TestHTTPClient: HTTPClientInterface {
  struct State {
    var requests: [HTTPRequest<Data?>] = []
    var streams: [ResponseCallbacks] = []
    var subscriptions: [Releash_Client_V1_StartStateSubscriptionRequest] = []
    var repositories: [Releash_Client_V1_WorkspaceRepositoryList] = []
    var layouts: [String: Releash_Client_V1_WorkspaceStateDto] = [:]
    var providerUsage = Releash_Client_V1_ProviderUsageSnapshot()
    var hookHealth = Releash_Client_V1_ProviderHookHealthSnapshot()
    var executionOverview = Releash_Client_V1_ExecutionOverviewDto()
    var daemonInfo = Releash_Client_V1_ServerInfo()
    var desktopSettings = Releash_Client_V1_DesktopSettings()
    var loginRegistration = Releash_Client_V1_LoginRegistrationResult()
    var cliInstallResult = Releash_Client_V1_InstallCliResponse()
    var externalEditor = Releash_Client_V1_ExternalEditorState()
    var providerAvailability = Releash_Client_V1_ProviderAvailabilitySnapshotResponse()
    var terminalStartupCommand = ""
    var workflows = Releash_Client_V1_ListWorkflowSummaryDto()
    var workflowDetails: [String: Releash_Client_V1_NullableWorkflowDto] = [:]
    var facets: [String: Releash_Client_V1_ListFacetSummaryDto] = [:]
    var diagnostics = Releash_Client_V1_DiagnosticReport()
    var workflowConfig = Releash_Client_V1_WorkflowSection()
    var releashBases: [String: Releash_Client_V1_Nullablestring] = [:]
    var branches: [String: Releash_Client_V1_ListBranchDto] = [:]
    var notionConfigs: [String: Releash_Client_V1_NullableNotionRepoConfigView] = [:]
    var notionValidation = Releash_Client_V1_NotionValidationResultView()
    var deferNotionValidation = false
    var pendingNotionValidation: (@Sendable (HTTPResponse) -> Void)?
    var collapsed: [String: Bool] = [:]
    var terminalReplay = ""
    var artifacts: [String: Releash_Client_V1_WorkflowOutputView] = [:]
    var nodeDetails: [String: Releash_Client_V1_WorkspaceNodeDetailDto] = [:]
    var sessions: [String: Releash_Client_V1_AgentSessionItemDto] = [:]
    var sessionTabs: [String: Releash_Client_V1_SessionTabSnapshot] = [:]
    var reviewThreads: [String: Releash_Client_V1_ListReviewThreadDto] = [:]
    var history = Releash_Client_V1_AgentSessionHistoryPageDto()
    var failedMethod: String?
    var failedMethodCode: Code = .internalError
    var failureDetail: ConnectError.Detail?
    var failedSubscriptionTargets: [String: Int] = [:]
    var subscriptionFailureCode: Code = .internalError
    var pendingCreation: (@Sendable (HTTPResponse) -> Void)?
    var deferOpenSession = false
    var pendingOpenSession: (@Sendable (HTTPResponse) -> Void)?
    var deferCreation = false
    var deferSave = false
    var pendingSave: (@Sendable (HTTPResponse) -> Void)?
    var deferredTerminalMethod: String?
    var pendingTerminal: (@Sendable (HTTPResponse) -> Void)?
  }
  let state = Mutex(State())
  let discoveryFile = FileManager.default.temporaryDirectory.appendingPathComponent(
    UUID().uuidString)
  deinit { try? FileManager.default.removeItem(at: discoveryFile) }

  func unary(
    request: HTTPRequest<Data?>,
    onMetrics: @escaping @Sendable (HTTPMetrics) -> Void,
    onResponse: @escaping @Sendable (HTTPResponse) -> Void
  ) -> Cancelable {
    do {
      let method = request.url.lastPathComponent
      let failureCode: Code? = state.withLock { state in
        state.requests.append(request)
        if method == "StartStateSubscription",
          let subscription = try? Releash_Client_V1_StartStateSubscriptionRequest(
            serializedBytes: request.message ?? Data()),
          state.failedSubscriptionTargets[subscription.target, default: 0] > 0
        {
          state.failedSubscriptionTargets[subscription.target, default: 0] -= 1
          return state.subscriptionFailureCode
        }
        return state.failedMethod == method ? state.failedMethodCode : nil
      }
      if let failureCode {
        let detail = state.withLock { $0.failureDetail }
        onResponse(
          HTTPResponse(
            code: failureCode, headers: [:], message: nil, trailers: [:],
            error: ConnectError(
              code: failureCode, message: "\(method) failed", details: detail.map { [$0] } ?? []),
            tracingInfo: nil))
        return Cancelable(cancel: {})
      }
      if state.withLock({ $0.deferredTerminalMethod == method }) {
        state.withLock { $0.pendingTerminal = onResponse }
        return Cancelable(cancel: {})
      }
      if method == "OpenAgentSession", state.withLock({ $0.deferOpenSession }) {
        state.withLock { $0.pendingOpenSession = onResponse }
        return Cancelable(cancel: {})
      }
      if method == "CreateWorktrees", state.withLock({ $0.deferCreation }) {
        state.withLock { $0.pendingCreation = onResponse }
        return Cancelable(cancel: {})
      }
      if method == "ValidateNotionConfig", state.withLock({ $0.deferNotionValidation }) {
        state.withLock { $0.pendingNotionValidation = onResponse }
        return Cancelable(cancel: {})
      }
      if ["UpdateProviderExecutable", "ResetProviderExecutable", "RefreshProviderAvailability"]
        .contains(method), state.withLock({ $0.deferSave })
      {
        state.withLock { $0.pendingSave = onResponse }
        return Cancelable(cancel: {})
      }
      var savedWorkspaceState = Data()
      if method == "StartStateSubscription" {
        let subscription = try Releash_Client_V1_StartStateSubscriptionRequest(
          serializedBytes: request.message ?? Data())
        let (payload, stream) = state.withLock { state in
          state.subscriptions.append(subscription)
          var payload = Releash_Client_V1_StatePayload()
          if subscription.target == "workspace-terminal" || subscription.target == "terminal" {
            payload.terminal.snapshot.replay = state.terminalReplay
            payload.terminal.snapshot.cols = 80
            payload.terminal.snapshot.rows = 24
            payload.terminal.snapshot.processedReportUnits = 5000
            payload.terminal.snapshot.status.cwd = subscription.args[0]
            payload.terminal.snapshot.status.hasForegroundProcess_p = false
          } else if subscription.target == "node-detail" {
            if let value = state.nodeDetails[subscription.args[1]] {
              payload.nodeDetail.value = value
            }
          } else if subscription.target == "node-artifact" {
            if let value = state.artifacts[subscription.args[0]] {
              payload.workflowOutput.value = value
            }
          } else if subscription.target == "session-tab" {
            payload.sessionTab = state.sessionTabs[subscription.args[0]] ?? .init()
          } else if subscription.target == "review-threads" {
            payload.reviewThreads = state.reviewThreads[subscription.args[0]] ?? .init()
          } else if subscription.target == "agent-session" {
            payload.agentSession = .init()
            if let value = state.sessions[subscription.args[0]] {
              payload.agentSession.value = value
            }
          } else if subscription.target == "session-history" {
            payload.sessionHistory = state.history
          } else if subscription.target == "workspaces" {
            payload.workspaces.repositories.items = state.repositories
          } else if subscription.target == "provider-usage" {
            payload.providerUsage = state.providerUsage
          } else if subscription.target == "provider-hook-health" {
            payload.providerHookHealth = state.hookHealth
          } else if subscription.target == "execution-overview" {
            payload.executionOverview = state.executionOverview
          } else if subscription.target == "daemon-info" {
            payload.daemonInfo = state.daemonInfo
          } else if subscription.target == "desktop-settings" {
            payload.desktopSettings = state.desktopSettings
          } else if subscription.target == "external-editor" {
            payload.externalEditor = state.externalEditor
          } else if subscription.target == "provider-availability" {
            payload.providerAvailability = state.providerAvailability
          } else if subscription.target == "terminal-startup-command" {
            payload.terminalStartupCommand.value = state.terminalStartupCommand
          } else if subscription.target == "workflows" {
            payload.workflows = state.workflows
          } else if subscription.target == "workflow" {
            payload.workflow = state.workflowDetails[subscription.args[0]] ?? .init()
          } else if subscription.target == "facets" {
            payload.facets = state.facets[subscription.args[0]] ?? .init()
          } else if subscription.target == "diagnostics" {
            payload.diagnostics = state.diagnostics
          } else if subscription.target == "workflow-config" {
            payload.workflowConfig = state.workflowConfig
          } else if subscription.target == "releash-base" {
            payload.releashBase = state.releashBases[subscription.args[0]] ?? .init()
          } else if subscription.target == "branches" {
            payload.branches = state.branches[subscription.args[0]] ?? .init()
          } else if subscription.target == "notion-config" {
            payload.notionConfig = state.notionConfigs[subscription.args[0]] ?? .init()
          } else if subscription.target == "repository-group-state" {
            payload.repositoryGroupState.collapsed =
              state.collapsed[subscription.args[0], default: false]
          } else if subscription.target == "workspace-state" {
            var nullable = Releash_Client_V1_NullableWorkspaceStateDto()
            if let layout = state.layouts[subscription.args[0]] { nullable.value = layout }
            payload.workspaceState = nullable
          }
          return (payload, state.streams.last)
        }
        var event = Releash_Client_V1_StateSubscriptionEvent()
        event.subscriptionID = subscription.subscriptionID
        event.snapshot = payload
        if let stream { try Self.send(event, on: stream) }
      } else if method == "SaveRepositoryGroupState" {
        let save = try Releash_Client_V1_SaveRepositoryGroupStateRequest(
          serializedBytes: request.message ?? Data())
        state.withLock { $0.collapsed[save.repositoryPath] = save.collapsed }
      } else if method == "SaveWorkspaceState" {
        let save = try Releash_Client_V1_SaveWorkspaceStateRequest(
          serializedBytes: request.message ?? Data())
        let saved: Releash_Client_V1_WorkspaceStateDto? = state.withLock { state in
          let current = state.layouts[save.worktreeName]?.etag ?? "0"
          guard save.state.etag == current else { return nil }
          var stored = save.state
          stored.etag = String((Int(current) ?? 0) + 1)
          state.layouts[save.worktreeName] = stored
          return stored
        }
        guard let saved else {
          onResponse(
            HTTPResponse(
              code: .aborted, headers: [:], message: nil, trailers: [:],
              error: ConnectError(code: .aborted, message: "stale workspace state"),
              tracingInfo: nil))
          return Cancelable(cancel: {})
        }
        savedWorkspaceState = try saved.serializedData()
        if state.withLock({ $0.deferSave }) {
          state.withLock { $0.pendingSave = onResponse }
          return Cancelable(cancel: {})
        }
      }
      var response = savedWorkspaceState
      if ["CreateAgentSession", "RestoreAgentSession", "ResumeAgentSessionHistoryCandidate"]
        .contains(method)
      {
        var selection = Releash_Client_V1_SessionSelection()
        selection.agentSessionID = "agent-session"
        selection.nodeID = "session-tree"
        response = try selection.serializedData()
      } else if method == "StartWorkflow" {
        var result = Releash_Client_V1_ResultString()
        result.value = "workflow-execution"
        response = try result.serializedData()
      } else if method == "ValidateNotionConfig" {
        response = try state.withLock { try $0.notionValidation.serializedData() }
      } else if method == "CheckLoginRegistration" {
        response = try state.withLock { try $0.loginRegistration.serializedData() }
      } else if method == "InstallCli" {
        response = try state.withLock { try $0.cliInstallResult.serializedData() }
      }
      onResponse(
        HTTPResponse(
          code: .ok, headers: ["content-type": ["application/proto"]], message: response,
          trailers: [:], error: nil, tracingInfo: .init(httpStatus: 200)))
    } catch {
      onResponse(Self.failure(error.localizedDescription))
    }
    return Cancelable(cancel: {})
  }
  func stream(request: HTTPRequest<Data?>, responseCallbacks: ResponseCallbacks)
    -> RequestCallbacks<Data>
  {
    state.withLock { state in
      state.requests.append(request)
      state.streams.append(responseCallbacks)
    }
    return RequestCallbacks(
      cancel: { Task { responseCallbacks.receiveClose(.canceled, [:], nil) } },
      sendData: { _ in
        responseCallbacks.receiveResponseHeaders(["content-type": ["application/connect+proto"]])
        var event = Releash_Client_V1_StateSubscriptionEvent()
        event.ready = .init()
        do { try Self.send(event, on: responseCallbacks) } catch {
          responseCallbacks.receiveClose(.internalError, [:], error)
        }
      }, sendClose: {})
  }
  static func failure(_ message: String) -> HTTPResponse {
    HTTPResponse(
      code: .internalError, headers: [:], message: nil, trailers: [:],
      error: ConnectError(code: .internalError, message: message), tracingInfo: nil)
  }
  static func send(_ event: Releash_Client_V1_StateSubscriptionEvent, on stream: ResponseCallbacks)
    throws
  {
    let bytes = try event.serializedData()
    let count = UInt32(bytes.count)
    var framed = Data([
      0, UInt8(truncatingIfNeeded: count >> 24), UInt8(truncatingIfNeeded: count >> 16),
      UInt8(truncatingIfNeeded: count >> 8), UInt8(truncatingIfNeeded: count),
    ])
    framed.append(bytes)
    stream.receiveResponseData(framed)
  }
  func publish(_ payload: Releash_Client_V1_StatePayload, target: String, args: [String]? = nil)
    throws
  {
    let (subscriptions, stream) = state.withLock { state in
      switch payload.value {
      case .desktopSettings(let value): state.desktopSettings = value
      case .executionOverview(let value): state.executionOverview = value
      case .daemonInfo(let value): state.daemonInfo = value
      case .workspaces(let value): state.repositories = value.repositories.items
      case .sessionHistory(let value): state.history = value
      case .externalEditor(let value): state.externalEditor = value
      case .providerAvailability(let value): state.providerAvailability = value
      case .terminalStartupCommand(let value): state.terminalStartupCommand = value.value
      case .workflowConfig(let value): state.workflowConfig = value
      case .releashBase(let value):
        if let path = args?.first { state.releashBases[path] = value }
      case .branches(let value):
        if let path = args?.first { state.branches[path] = value }
      case .notionConfig(let value):
        if let path = args?.first { state.notionConfigs[path] = value }
      case .workflows(let value): state.workflows = value
      case .workflow(let value):
        if let name = args?.first { state.workflowDetails[name] = value }
      case .facets(let value):
        if let kind = args?.first { state.facets[kind] = value }
      case .diagnostics(let value): state.diagnostics = value
      case .agentSession(let value):
        if value.hasValue { state.sessions[value.value.id] = value.value }
      case .nodeDetail(let value):
        if value.hasValue { state.nodeDetails[value.value.id] = value.value }
      case .sessionTab(let value):
        if let id = args?.first { state.sessionTabs[id] = value }
      case .reviewThreads(let value):
        if let path = args?.first { state.reviewThreads[path] = value }
      default: break
      }
      return (
        state.subscriptions.filter { $0.target == target && (args == nil || $0.args == args) },
        state.streams.last
      )
    }
    guard let stream else { return }
    for subscription in subscriptions {
      var event = Releash_Client_V1_StateSubscriptionEvent()
      event.subscriptionID = subscription.subscriptionID
      event.snapshot = payload
      try Self.send(event, on: stream)
    }
  }
  func requests(_ method: String) -> [HTTPRequest<Data?>] {
    state.withLock { $0.requests.filter { $0.url.lastPathComponent == method } }
  }
}

func testProviderAvailability() -> Releash_Client_V1_ProviderAvailabilitySnapshotResponse {
  .with {
    $0.providers.items = [
      .with {
        $0.provider = "claude"
        $0.displayName = "Claude Code"
        $0.defaultExecutable = "claude"
        $0.configuredExecutable = "/custom/claude"
        $0.effectiveExecutable = "/custom/claude"
        $0.available = true
        $0.resolvedExecutable = "/resolved/claude"
      },
      .with {
        $0.provider = "codex"
        $0.displayName = "Codex"
        $0.defaultExecutable = "codex"
        $0.effectiveExecutable = "codex"
        $0.available = false
        $0.unavailableReason = "not_found"
      },
    ]
  }
}

func testReviewThread(_ id: String, resolved: Bool = false) -> Releash_Client_V1_ReviewThreadDto {
  var thread = Releash_Client_V1_ReviewThreadDto()
  thread.id = id
  thread.state.value = resolved ? .resolved : .open
  thread.canResolve = !resolved
  var comment = Releash_Client_V1_ReviewCommentDto()
  comment.id = "comment:\(id)"
  comment.content = "## \(id)\n\n- **確認**\n\n```swift\nlet value = 1\n```"
  comment.author.displayName = "Reviewer"
  comment.createdAt = 1_700_000_000
  thread.comments.items = [comment]
  return thread
}

@MainActor func eventually(
  _ condition: () -> Bool, file: StaticString = #filePath, line: UInt = #line
) async throws {
  let deadline = ContinuousClock.now + .seconds(5)
  while !condition() && ContinuousClock.now < deadline {
    try await Task.sleep(for: .milliseconds(5))
  }
  XCTAssertTrue(condition(), file: file, line: line)
}

@MainActor func testModel(_ http: TestHTTPClient) async throws -> AppModel {
  try Data(
    #"{"port":12345,"token":"operator-test","daemon_id":"test","pid":1,"process_started_at":"1"}"#
      .utf8
  ).write(to: http.discoveryFile)
  let status = try JSONSerialization.data(withJSONObject: [
    "running": true, "compatibility": "compatible", "release_compatibility": "compatible",
    "discovery_file": http.discoveryFile.path,
  ])
  let model = AppModel(
    cli: CLI(
      executable: URL(fileURLWithPath: "/test/releash"),
      run: { _, args in
        XCTAssertEqual(args, ["status", "--json"])
        return status
      }), httpClient: http)
  await model.start()
  try await eventually { model.connected }
  return model
}

func testWorkflowSettings(_ http: TestHTTPClient) {
  http.state.withLock { state in
    state.workflows.items = [
      .with {
        $0.name = "builtin-flow"
        $0.description_p = "組み込みの定義"
        $0.builtin = true
        $0.sourceFormat.value = .yaml
      },
      .with {
        $0.name = "01_author-spec"
        $0.description_p = "利用者の定義"
        $0.isRunning = true
        $0.sourceFormat.value = .lua
      },
      .with {
        $0.name = "broken"
        $0.readError = "定義を読み取れません"
      },
    ]
    let nodes: [Releash_Client_V1_NodeDefinitionDto] = [
      .with {
        $0.name = "review"
        $0.kind.value = .session
        $0.session.provider.value = .codex
        $0.session.model = "test-model"
        $0.session.permission = "read-only"
        $0.session.facets.policy = "coding"
        $0.session.facets.knowledge.items = ["guide"]
        $0.session.facets.instruction = "implement"
        $0.artifact = "report"
        $0.input.items = [
          .with {
            $0.name = "source"
            $0.contract = "ReviewInput"
          }
        ]
        $0.completion.require.value = .approval
        $0.completion.delegate.child = "worker"
        $0.completion.delegate.maxIterations = 2
        $0.completion.delegate.when.ref = "retry"
        $0.completion.delegate.inputs.items = [
          .with {
            $0.parameter = "task"
            $0.source = "input.task"
          }
        ]
        $0.worktree.value = .isolated
      },
      .with {
        $0.name = "check"
        $0.kind.value = .command
        $0.command = "echo checked"
      },
      .with {
        $0.name = "parallel"
        $0.kind.value = .fanout
        $0.fanout.children.items = [.with { $0.name = "worker" }]
        $0.fanout.items.artifactField = "tasks"
      },
      .with {
        $0.name = "serial"
        $0.kind.value = .sequence
        $0.sequence.entry = "first"
        $0.sequence.children.items = [.with { $0.name = "first" }]
      },
    ]
    for summary in state.workflows.items where !summary.hasReadError {
      state.workflowDetails[summary.name] = .with {
        $0.value.name = summary.name
        $0.value.description_p = summary.description_p
        $0.value.builtin = summary.builtin
        $0.value.nodes.items = nodes
      }
    }
    for kind in ["policy", "knowledge", "instruction"] {
      state.facets[kind] = .with {
        $0.items = [
          .with {
            $0.kind = kind
            $0.key = "builtin-facet"
            $0.builtin = true
            $0.description_p = "組み込みFacet"
          },
          .with {
            $0.kind = kind
            $0.key = "coding"
            $0.description_p = "共有Facetの説明"
          },
          .with {
            $0.kind = kind
            $0.key = "guide"
            $0.description_p = "知識の説明"
          },
          .with {
            $0.kind = kind
            $0.key = "implement"
            $0.description_p = "指示の説明"
          },
        ]
      }
    }
    state.diagnostics.items.items = [
      .with {
        $0.code = "WFE001"
        $0.workflowName = "01_author-spec"
        $0.severity.value = .error
        $0.stage.value = .parseShape
        $0.span.source = "01_author-spec.lua"
        $0.span.startLine = 2
        $0.span.startCol = 3
        $0.span.endLine = 4
        $0.span.endCol = 5
        $0.message = "定義のエラー"
      },
      .with {
        $0.code = "WFI000"
        $0.workflowName = "builtin-flow"
        $0.severity.value = .info
        $0.stage.value = .resolve
        $0.message = "組み込みの情報"
      },
      .with {
        $0.code = "WFE002"
        $0.workflowName = "broken"
        $0.message = "解釈できない定義"
      },
      .with {
        $0.code = "WFE003"
        $0.facetKind = "policy"
        $0.facetKey = "coding"
        $0.message = "Facetのエラー"
      },
    ]
    state.diagnostics.workflowSummaries.entries["01_author-spec"] = .with {
      $0.errorCount = 1
      $0.infoCount = 2
    }
    state.diagnostics.workflowSummaries.entries["builtin-flow"] = .with { $0.infoCount = 1 }
    state.diagnostics.facetSummaries.entries["policy/coding"] = .with {
      $0.errorCount = 3
      $0.infoCount = 4
    }
    state.diagnostics.facetUsage.entries["policy/coding"] = .with {
      $0.items = [
        .with {
          $0.workflowName = "01_author-spec"
          $0.nodeName = "review"
          $0.slot = "policy"
        },
        .with {
          $0.workflowName = "builtin-flow"
          $0.nodeName = "review"
          $0.slot = "policy"
        },
      ]
    }
  }
}
