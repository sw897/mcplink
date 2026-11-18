import Foundation
import Security
import CryptoKit
import Darwin

// A deliberately small, separately deployed process. Never rebuild this helper
// as part of an ordinary App/CLI build: self-signed Keychain partitions use its
// CodeDirectory hash, even when a designated requirement stays unchanged.
let protocolVersion = 1
let limit = 16 * 1024 * 1024
let service = "io.mcplink.credentials"

func reply(_ object: [String: Any], _ code: Int32 = 0) -> Never {
    if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data("\n".utf8))
    }
    exit(code)
}
func fail(_ reason: String, _ status: OSStatus = errSecParam) -> Never {
    // Neither Security framework messages nor input are echoed: values may be secret.
    reply(["ok": false, "error": reason, "status": status], 1)
}
guard SecKeychainSetUserInteractionAllowed(false) == errSecSuccess else { fail("interaction_policy") }
var own: SecCode?
guard SecCodeCopySelf(SecCSFlags(), &own) == errSecSuccess, let own = own,
      SecCodeCheckValidity(own, SecCSFlags(), nil) == errSecSuccess else { fail("helper_signature") }
var parent: SecCode?
guard SecCodeCopyGuestWithAttributes(nil, [kSecGuestAttributePid as String: NSNumber(value: getppid())] as CFDictionary,
                                   SecCSFlags(), &parent) == errSecSuccess, let parent = parent,
      SecCodeCheckValidity(parent, SecCSFlags(), nil) == errSecSuccess else { fail("caller_signature") }
func identity(_ code: SecCode) -> (String, Data)? {
    var staticCode: SecStaticCode?
    guard SecCodeCopyStaticCode(code, SecCSFlags(), &staticCode) == errSecSuccess,
          let staticCode = staticCode else { return nil }
    var info: CFDictionary?
    guard SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: UInt32(kSecCSSigningInformation)), &info) == errSecSuccess,
          let dict = info as? [String: Any], let id = dict[kSecCodeInfoIdentifier as String] as? String,
          let certs = dict[kSecCodeInfoCertificates as String] as? [SecCertificate], let cert = certs.first else { return nil }
    return (id, SecCertificateCopyData(cert) as Data)
}
guard let (helperID, helperCert) = identity(own), helperID == "io.mcplink.keychain",
      let (callerID, callerCert) = identity(parent), callerCert == helperCert,
      ["io.mcplink.cli", "io.mcplink.desktop"].contains(callerID) else { fail("caller_identity") }

let arguments = Array(CommandLine.arguments.dropFirst())
guard arguments.isEmpty || arguments == ["--authorize"] else { fail("invalid_arguments") }
var bytes = Data()
while let part = try? FileHandle.standardInput.read(upToCount: 65536), !part.isEmpty {
    bytes.append(part)
    if bytes.count > limit { fail("input_too_large") }
}
guard let request = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any],
      Set(request.keys).isSubset(of: ["version", "operation", "dataDirectory", "key", "value"]),
      request["version"] as? Int == protocolVersion,
      let operation = request["operation"] as? String,
      let directory = request["dataDirectory"] as? String, (directory as NSString).isAbsolutePath,
      let key = request["key"] as? String, key.utf8.count <= 256,
      key.range(of: #"^(runtime-key|(?:definition|plan|change|oauth)/[A-Za-z0-9._-]+)$"#, options: .regularExpression) != nil,
      ["get", "set", "delete", "authorize"].contains(operation),
      (operation == "set") == (request["value"] is String),
      arguments.isEmpty == (operation != "authorize") else { fail("invalid_request") }
let namespace = SHA256.hash(data: Data(directory.utf8)).map { String(format: "%02x", $0) }.joined().prefix(20)
var query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                          kSecAttrService as String: service,
                          kSecAttrAccount as String: "\(namespace)/\(key)"]
// Integration tests use a private, explicitly marked directory and a separate
// Keychain. This does not change the default/search list or query real items.
var fixtureKeychain: SecKeychain?
let fixtureMarker = URL(fileURLWithPath: directory).appendingPathComponent(".mcplink-keychain-test-fixture")
if FileManager.default.fileExists(atPath: fixtureMarker.path) {
    let keychainPath = URL(fileURLWithPath: directory).appendingPathComponent("fixture.keychain-db").path
    guard SecKeychainOpen(keychainPath, &fixtureKeychain) == errSecSuccess,
          let keychain = fixtureKeychain else { fail("fixture_keychain") }
    query[kSecMatchSearchList as String] = [keychain]
}
if operation == "authorize" {
    guard SecKeychainSetUserInteractionAllowed(true) == errSecSuccess else { fail("interaction_policy") }
}
switch operation {
case "get", "authorize":
    var q = query
    q[kSecReturnData as String] = true
    q[kSecMatchLimit as String] = kSecMatchLimitOne
    var result: CFTypeRef?
    let status = SecItemCopyMatching(q as CFDictionary, &result)
    if status == errSecItemNotFound { reply(["ok": true, "found": false]) }
    guard status == errSecSuccess, let data = result as? Data else { fail("keychain_access", status) }
    if operation == "authorize" { reply(["ok": true, "found": true]) }
    guard let value = String(data: data, encoding: .utf8) else { fail("invalid_credential_encoding") }
    reply(["ok": true, "found": true, "value": value])
case "set":
    let value = Data((request["value"] as! String).utf8)
    let changes = [kSecValueData as String: value]
    var status = SecItemUpdate(query as CFDictionary, changes as CFDictionary)
    if status == errSecItemNotFound {
        var addition = query.merging(changes) { _, new in new }
        addition.removeValue(forKey: kSecMatchSearchList as String)
        if let keychain = fixtureKeychain { addition[kSecUseKeychain as String] = keychain }
        status = SecItemAdd(addition as CFDictionary, nil)
    }
    guard status == errSecSuccess else { fail("keychain_access", status) }
    reply(["ok": true])
case "delete":
    let status = SecItemDelete(query as CFDictionary)
    guard status == errSecSuccess || status == errSecItemNotFound else { fail("keychain_access", status) }
    reply(["ok": true])
default: fail("invalid_request")
}
