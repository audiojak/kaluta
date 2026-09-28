import Foundation
import Testing
@testable import OpenAGC

/// Runs against the real Keychain under a throwaway service name.
struct KeychainTests {
    @Test func setGetUpdateDeleteRoundTrip() throws {
        let store = KeychainSecretStore(service: "ai.actual.openagc.tests.\(UUID().uuidString)")
        #expect(try store.get("k") == nil)
        try store.set("k", "first")
        #expect(try store.get("k") == "first")
        try store.set("k", "second")
        #expect(try store.get("k") == "second")
        try store.delete("k")
        #expect(try store.get("k") == nil)
        try store.delete("k") // deleting a missing item is fine
    }

    @Test func servicesAreIsolated() throws {
        let a = KeychainSecretStore(service: "ai.actual.openagc.tests.\(UUID().uuidString)")
        let b = KeychainSecretStore(service: "ai.actual.openagc.tests.\(UUID().uuidString)")
        try a.set("shared-key", "a")
        defer { try? a.delete("shared-key") }
        #expect(try b.get("shared-key") == nil)
    }

    @Test func deleteAllEmptiesOnlyItsService() throws {
        let a = KeychainSecretStore(service: "ai.actual.openagc.tests.\(UUID().uuidString)")
        let b = KeychainSecretStore(service: "ai.actual.openagc.tests.\(UUID().uuidString)")
        try a.set("one", "1")
        try a.set("two", "2")
        try b.set("one", "b")
        defer { try? b.delete("one") }
        try a.deleteAll()
        #expect(try a.get("one") == nil && a.get("two") == nil)
        #expect(try b.get("one") == "b")
    }

    @Test func everythingDefaultsToScratchSecretsAndPreferencesUnderTests() {
        #expect(CoreClient.defaultSecrets().service == CoreClient.testSecretsService)
        #expect(CoreClient.appDefaults() !== UserDefaults.standard)
        #expect(CoreClient.appDefaults() === CoreClient.appDefaults(), "one suite per process")
    }
}
