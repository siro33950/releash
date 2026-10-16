import Bonsplit
import SwiftUI

struct SessionTabPane: View {
  @Bindable var model: AppModel
  let tabID: TabID
  let sessionID: String
  let focused: Bool
  @State private var snapshot = Releash_Client_V1_SessionTabSnapshot()
  @State private var subscription: UUID?
  @State private var failure: String?

  var body: some View {
    VStack {
      if let failure { Text(failure).foregroundStyle(.red).textSelection(.enabled) }
      switch snapshot.session.availability {
      case .open:
        TerminalPane(
          session: model.terminal(
            path: snapshot.session.workspaceWorktreePath, id: sessionID, agent: true),
          focused: focused)
      case .paused:
        Text("Sessionは停止しています")
        if snapshot.node.capabilities.canResumeSession {
          Button("Resume") {
            model.resumeNode(path: snapshot.session.workspaceWorktreePath, id: snapshot.node.id)
          }
        }
      case .archived: Text("SessionはArchiveされています")
      case .indeterminate: Text("Sessionを開けるか判定できません")
      default: ProgressView()
      }
    }.frame(maxWidth: .infinity, maxHeight: .infinity)
      .task {
        subscription = model.subscribe("session-tab", [sessionID]) { payload in
          if case .sessionTab(let value) = payload.value {
            if snapshot.hasSession && (!value.hasSession || value.session.availability != .open) {
              model.releaseAgentTerminal(
                path: snapshot.session.workspaceWorktreePath, id: sessionID)
            }
            snapshot = value
            model.panes.updateSession(tabID, snapshot: value)
          }
        }
        guard let client = model.client else { return }
        var request = Releash_Client_V1_OpenAgentSessionRequest()
        request.agentSessionID = sessionID
        request.rows = UIStyle.terminalRows
        request.cols = UIStyle.terminalColumns
        request.callerRequestID = UUID().uuidString
        do {
          _ = try await client.openAgentSession(request: request, headers: model.headers).result
            .get()
        } catch { failure = model.failureMessage(error) }
      }
      .onDisappear {
        if let subscription { model.unsubscribe(subscription) }
        subscription = nil
        model.panes.sessionSnapshots.removeValue(forKey: tabID)
        if snapshot.hasSession {
          model.releaseAgentTerminal(path: snapshot.session.workspaceWorktreePath, id: sessionID)
        }
      }
  }
}

struct SessionTabMenu: View {
  @Bindable var model: AppModel
  let tabID: TabID

  var body: some View {
    if let snapshot = model.panes.sessionSnapshots[tabID], snapshot.hasSession, snapshot.hasNode {
      if snapshot.node.capabilities.canRename {
        Button("Rename") {
          model.panes.sessionName = snapshot.node.title
          model.panes.renamingSession = snapshot
        }
      }
      if snapshot.session.operations.canArchive {
        Button("Archive") {
          model.requestArchiveSessionTab(snapshot)
        }
      }
    }
  }
}

extension AppModel {
  func requestArchiveSessionTab(_ snapshot: Releash_Client_V1_SessionTabSnapshot) {
    guard snapshot.hasNode, snapshot.session.operations.canArchive else { return }
    if snapshot.node.canAbort {
      panes.archivingSession = snapshot
    } else {
      archiveSessionTab(snapshot)
    }
  }

  func confirmArchiveSessionTab() {
    if let snapshot = panes.archivingSession { archiveSessionTab(snapshot) }
    panes.archivingSession = nil
  }

  func renameSessionTab(_ snapshot: Releash_Client_V1_SessionTabSnapshot, name: String) {
    guard let client else { return }
    var request = Releash_Client_V1_RenameWorkspaceSessionNodeRequest()
    request.worktreePath = snapshot.session.workspaceWorktreePath
    request.nodeID = snapshot.node.id
    request.name = name
    perform {
      _ = try await client.renameWorkspaceSessionNode(request: request, headers: self.headers)
        .result.get()
    }
  }

  func archiveSessionTab(_ snapshot: Releash_Client_V1_SessionTabSnapshot) {
    guard let client else { return }
    var request = Releash_Client_V1_ArchiveWorkspaceWorkflowExecutionRequest()
    request.worktreePath = snapshot.session.workspaceWorktreePath
    request.executionID = snapshot.session.treeLocation.treeID
    perform {
      _ = try await client.archiveWorkspaceWorkflowExecution(
        request: request, headers: self.headers
      ).result.get()
    }
  }
}
