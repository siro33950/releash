import Foundation

struct StateSubscriptionIDs {
  private var keys: [String: String] = [:]
  private var ids: [String: String] = [:]

  mutating func reserve(_ key: String) -> (id: String, previous: String?) {
    let previous = remove(key)
    let id = UUID().uuidString
    keys[id] = key
    ids[key] = id
    return (id, previous)
  }
  mutating func remove(_ key: String) -> String? {
    guard let id = ids.removeValue(forKey: key) else { return nil }
    keys.removeValue(forKey: id)
    return id
  }
  func key(for id: String) -> String? { keys[id] }
  mutating func reset() {
    keys.removeAll()
    ids.removeAll()
  }
}
