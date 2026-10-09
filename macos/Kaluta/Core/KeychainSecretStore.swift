import Foundation
import Security

/// Kaluta's secrets in the macOS Keychain (spec §12): generic passwords
/// under one service, readable only after first unlock, never synced.
///
/// Uses the data-protection keychain when the app is signed with a team
/// (release builds). Ad-hoc signed development builds lack the entitlement
/// (errSecMissingEntitlement), so they fall back to the login keychain.
final class KeychainSecretStore: @unchecked Sendable {
    let service: String
    private let lock = NSLock()
    private var useDataProtection: Bool?

    /// The user's own items: Kaluta's, and OpenAGC's from before the
    /// project was renamed, which the first launch copies and never changes.
    static let realServices: Set<String> = ["org.kaluta.Kaluta", "ai.actual.openagc"]

    init(service: String = "org.kaluta.Kaluta") {
        self.service = service
    }

    func get(_ key: String) throws(KeychainError) -> String? {
        var result: CFTypeRef?
        let status = run {
            var query = baseQuery(key)
            query[kSecReturnData as String] = true
            query[kSecMatchLimit as String] = kSecMatchLimitOne
            return SecItemCopyMatching(query as CFDictionary, &result)
        }
        switch status {
        case errSecSuccess:
            guard let data = result as? Data, let value = String(data: data, encoding: .utf8) else {
                throw KeychainError(status: errSecDecode, operation: "decode")
            }
            return value
        case errSecItemNotFound:
            // Without a team-signed build the data-protection keychain can
            // answer "not found" rather than "missing entitlement", while
            // earlier writes went to the login keychain. Look there too,
            // and stay there if the item is found.
            guard dataProtection else { return nil }
            var legacy = baseQuery(key)
            legacy.removeValue(forKey: kSecUseDataProtectionKeychain as String)
            legacy[kSecReturnData as String] = true
            legacy[kSecMatchLimit as String] = kSecMatchLimitOne
            var found: CFTypeRef?
            guard SecItemCopyMatching(legacy as CFDictionary, &found) == errSecSuccess,
                  let data = found as? Data, let value = String(data: data, encoding: .utf8) else { return nil }
            lock.lock()
            useDataProtection = false
            lock.unlock()
            return value
        default:
            throw KeychainError(status: status, operation: "read")
        }
    }

    func set(_ key: String, _ value: String) throws(KeychainError) {
        let data = Data(value.utf8)
        let update = run {
            SecItemUpdate(baseQuery(key) as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        }
        if update == errSecSuccess { return }
        guard update == errSecItemNotFound else { throw KeychainError(status: update, operation: "update") }
        let status = run {
            var add = baseQuery(key)
            add[kSecValueData as String] = data
            add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
            add[kSecAttrLabel as String] = "Kaluta: \(key)"
            return SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else { throw KeychainError(status: status, operation: "add") }
    }

    func delete(_ key: String) throws(KeychainError) {
        let status = run { SecItemDelete(baseQuery(key) as CFDictionary) }
        guard status == errSecSuccess || status == errSecItemNotFound else {
            throw KeychainError(status: status, operation: "delete")
        }
    }

    /// The names of every item under this service, in either keychain.
    /// Reads no secret, so it never asks to unlock an item.
    func keys() -> [String] {
        var names = Set<String>()
        for dataProtection in [true, false] {
            var q: [String: Any] = [
                kSecClass as String: kSecClassGenericPassword,
                kSecAttrService as String: service,
                kSecReturnAttributes as String: true,
                kSecMatchLimit as String: kSecMatchLimitAll,
            ]
            if dataProtection { q[kSecUseDataProtectionKeychain as String] = true }
            var result: CFTypeRef?
            guard SecItemCopyMatching(q as CFDictionary, &result) == errSecSuccess,
                  let items = result as? [[String: Any]] else { continue }
            for item in items {
                if let name = item[kSecAttrAccount as String] as? String { names.insert(name) }
            }
        }
        return names.sorted()
    }

    /// Delete every item under this service (the test host's, between
    /// runs). Never used on the app's own service.
    func deleteAll() throws(KeychainError) {
        precondition(!Self.realServices.contains(service), "never empty the user's own Keychain items")
        // Both keychains: items may sit in the login keychain from runs
        // that fell back to it, while the data-protection one answers
        // "not found". The login keychain deletes one item per call.
        for dataProtection in [true, false] {
            for _ in 0..<1_000 {
                var q: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service]
                if dataProtection { q[kSecUseDataProtectionKeychain as String] = true }
                let status = SecItemDelete(q as CFDictionary)
                if status == errSecItemNotFound || (dataProtection && status == errSecMissingEntitlement) { break }
                guard status == errSecSuccess else { throw KeychainError(status: status, operation: "delete") }
            }
        }
    }

    private func baseQuery(_ key: String) -> [String: Any] {
        var q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
        ]
        if dataProtection { q[kSecUseDataProtectionKeychain as String] = true }
        return q
    }

    private var dataProtection: Bool {
        lock.lock()
        defer { lock.unlock() }
        return useDataProtection ?? true
    }

    /// Run a Keychain call. If the data-protection keychain reports a
    /// missing entitlement (ad-hoc signed builds), switch to the login
    /// keychain for the rest of the process and retry. Reads can report
    /// "not found" instead, so any call may trigger the switch.
    private func run(_ call: () -> OSStatus) -> OSStatus {
        let status = call()
        guard status == errSecMissingEntitlement else { return status }
        lock.lock()
        let wasDataProtection = useDataProtection ?? true
        useDataProtection = false
        lock.unlock()
        return wasDataProtection ? call() : status
    }
}

struct KeychainError: Error, CustomStringConvertible {
    let status: OSStatus
    let operation: String

    var description: String {
        let message = SecCopyErrorMessageString(status, nil) as String? ?? "OSStatus \(status)"
        return "Keychain \(operation) failed: \(message)"
    }
}
