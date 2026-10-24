import AppKit
import SwiftProtobuf
import SwiftTerm
import XCTest

@testable import Releash

final class TerminalInputModesTests: XCTestCase {
  private final class FocusOutput: TerminalDelegate {
    var data: [UInt8] = []
    func send(source: Terminal, data: ArraySlice<UInt8>) { self.data.append(contentsOf: data) }
  }

  private struct Fixture: Decodable {
    let name: String
    let output: String
    let checkpoint: Checkpoint
    struct Checkpoint: Decodable {
      let replay: String
      let cols: UInt32
      let rows: UInt32
    }
  }

  @MainActor func testBackendReplayPreservesActualInputAndSubsequentModeChanges() async throws {
    // Given
    let root = URL(fileURLWithPath: #filePath)
      .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent()
    let fixtures = try JSONDecoder().decode(
      [Fixture].self,
      from: Data(
        contentsOf: root.appending(path: "server/tests/fixtures/terminal-input-modes.json")))
    for fixture in fixtures {
      let liveFocus = FocusOutput()
      let restoredFocus = FocusOutput()
      let liveTerminal = Terminal(delegate: liveFocus)
      let restoredTerminal = Terminal(delegate: restoredFocus)
      liveTerminal.feed(text: fixture.output)
      restoredTerminal.feed(text: fixture.checkpoint.replay)
      let http = TestHTTPClient()
      let model = try await testModel(http)
      let live = model.terminal(path: "/repo", id: "live")
      let restored = model.terminal(path: "/repo", id: "restored")
      try await eventually {
        live.hasForegroundProcess != nil && restored.hasForegroundProcess != nil
      }
      let windows = [live, restored].map { session in
        let window = NSWindow(
          contentRect: NSRect(x: 0, y: 0, width: 640, height: 480), styleMask: .borderless,
          backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = session.screen
        return window
      }
      var event = Releash_Client_V1_TerminalEvent()
      event.snapshot.cols = fixture.checkpoint.cols
      event.snapshot.rows = fixture.checkpoint.rows
      event.snapshot.replay = fixture.output
      live.receive(event)
      event.snapshot.replay = fixture.checkpoint.replay
      restored.receive(event)
      // When / Then
      for change in [
        "", "\u{1b}[<u", "\u{1b}[?1049l\u{1b}[<u", "\u{1b}[?1049h\u{1b}[<2u",
        "\u{1b}[?1003;1006;1004;2004l\u{1b}[?1007l\u{1b}[=8;2u",
        "\u{1b}[?1002;1006;1004;2004h\u{1b}[?1007h\u{1b}[=3;3u",
      ] {
        live.screen.feed(text: change)
        event.output.data = change
        restored.receive(event)
        liveTerminal.feed(text: change)
        restoredTerminal.feed(text: change)
        liveFocus.data = []
        restoredFocus.data = []
        for terminal in [liveTerminal, restoredTerminal] {
          terminal.setTerminalFocus(false)
          terminal.setTerminalFocus(true)
        }
        XCTAssertEqual(restoredFocus.data, liveFocus.data)
        let expected = try await input(live, http: http)
        let actual = try await input(restored, http: http)
        XCTAssertEqual(actual, expected, "\(fixture.name), \(change.debugDescription)")
        if fixture.name == "codex" && change.isEmpty {
          XCTAssertTrue(actual.contains("\u{1b}[<64;"), "wheel must report a mouse event")
          XCTAssertTrue(actual.contains("\u{1b}[200~pasted\u{1b}[201~"))
          XCTAssertEqual(String(decoding: restoredFocus.data, as: UTF8.self), "\u{1b}[O\u{1b}[I")
          XCTAssertTrue(actual.contains("\u{1b}[?7u"))
        }
        if fixture.name == "disabled" && change.isEmpty {
          XCTAssertFalse(actual.contains("\u{1b}[A"))
          XCTAssertFalse(actual.contains("\u{1b}[B"))
          XCTAssertFalse(actual.contains("\u{1b}[200~"))
          XCTAssertTrue(restoredFocus.data.isEmpty)
        }
      }
      live.stop()
      restored.stop()
      await model.stop()
      for window in windows { window.close() }
    }
  }

  @MainActor private func input(_ session: TerminalSession, http: TestHTTPClient) async throws
    -> String
  {
    await withCheckedContinuation { continuation in
      DispatchQueue.main.async { continuation.resume() }
    }
    session.input("start-input")
    try await eventually { (try? self.writes(session, http: http).last?.data) == "start-input" }
    let start = try writes(session, http: http).count
    session.screen.feed(text: "\u{1b}[?u")
    session.screen.pasteText("pasted")
    let key = try XCTUnwrap(
      NSEvent.keyEvent(
        with: .keyDown, location: .zero, modifierFlags: .control, timestamp: 0, windowNumber: 0,
        context: nil, characters: "\u{3}", charactersIgnoringModifiers: "c", isARepeat: false,
        keyCode: 8))
    session.screen.keyDown(with: key)
    for units in [CGScrollEventUnit.line, .pixel] {
      let cg = try XCTUnwrap(
        CGEvent(
          scrollWheelEvent2Source: nil, units: units, wheelCount: 1,
          wheel1: units == .line ? 1 : 32, wheel2: 0,
          wheel3: 0))
      cg.location = CGPoint(x: 10, y: 10)
      session.screen.scrollWheel(with: try XCTUnwrap(NSEvent(cgEvent: cg)))
    }
    await withCheckedContinuation { continuation in
      DispatchQueue.main.async { continuation.resume() }
    }
    session.input("end-input")
    try await eventually { (try? self.writes(session, http: http).last?.data) == "end-input" }
    return try writes(session, http: http).dropFirst(start).map(\.data).joined()
  }

  @MainActor private func writes(_ session: TerminalSession, http: TestHTTPClient) throws
    -> [Releash_Client_V1_WriteTerminalSurfaceRequest]
  {
    try http.requests("WriteTerminalSurface").map {
      try Releash_Client_V1_WriteTerminalSurfaceRequest(serializedBytes: $0.message ?? Data())
    }.filter { $0.owner.workspaceTerminal.terminalID == session.id }
  }
}
