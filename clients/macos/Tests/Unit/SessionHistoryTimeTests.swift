import XCTest

@testable import Releash

final class SessionHistoryTimeTests: XCTestCase {
  func test_履歴時刻_暦の日と年の境界で表記を変える() throws {
    // Given
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = try XCTUnwrap(TimeZone(secondsFromGMT: 0))
    let now = try XCTUnwrap(
      calendar.date(from: DateComponents(year: 2026, month: 11, day: 15, hour: 1)))
    // When / Then
    for (seconds, expected) in [(30.0, "たった今"), (1500, "25 分前"), (7200, "昨日 23:00")] {
      XCTAssertEqual(
        historyTime(now.addingTimeInterval(-seconds), now: now, calendar: calendar), expected)
    }
    for (year, expected) in [(2026, "11月12日"), (2025, "2025年11月12日")] {
      let date = try XCTUnwrap(calendar.date(from: DateComponents(year: year, month: 11, day: 12)))
      XCTAssertEqual(historyTime(date, now: now, calendar: calendar), expected)
    }
  }
}
