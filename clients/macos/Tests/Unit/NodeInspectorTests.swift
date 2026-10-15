import SwiftUI
import XCTest

@testable import Releash

final class NodeInspectorTests: XCTestCase {

  func testNestedArtifactFieldsAndJSONPreserveArraysEscapedStringsAndScalars() throws {
    var value = Releash_Client_V1_WorkflowValue()
    value.objectValue.entries["name", default: .init()].stringValue.value = "quote\"\nline"
    var boolean = Releash_Client_V1_WorkflowValue()
    boolean.booleanValue.value = true
    var integer = Releash_Client_V1_WorkflowValue()
    integer.signedInteger.value = -2
    value.objectValue.entries["items", default: .init()].listValue.items = [boolean, integer]
    let decoded =
      try JSONSerialization.jsonObject(with: Data(value.displayJSON.utf8)) as? [String: Any]
    XCTAssertEqual(decoded?["name"] as? String, "quote\"\nline")
    XCTAssertEqual((decoded?["items"] as? [Any])?.count, 2)
    XCTAssertEqual(value.displayFields.map(\.name), ["items", "[0]", "[1]", "name"])
    XCTAssertEqual(value.displayFields.map(\.depth), [0, 1, 1, 0])
  }

  func testEmptyContainersAndNullKeepTheirValuesInTheTable() {
    var value = Releash_Client_V1_WorkflowValue()
    value.objectValue.entries["array", default: .init()].listValue = .init()
    value.objectValue.entries["object", default: .init()].objectValue = .init()
    value.objectValue.entries["null", default: .init()].nullValue = .init()
    XCTAssertEqual(value.displayFields.map(\.name), ["array", "null", "object"])
    XCTAssertEqual(value.displayFields.map(\.value), ["[]", "null", "{}"])
    value.objectValue = .init()
    XCTAssertEqual(value.displayFields.map(\.name), ["$"])
    XCTAssertEqual(value.displayFields.map(\.value), ["{}"])
  }

  func testArtifactFieldIDsEscapePointerElementsAtEveryDepth() {
    var value = Releash_Client_V1_WorkflowValue()
    value.objectValue.entries["a~/", default: .init()].objectValue.entries["b/~", default: .init()]
      .stringValue.value = "value"
    XCTAssertEqual(value.displayFields.map(\.id), ["/a~0~1", "/a~0~1/b~1~0"])
  }

}
