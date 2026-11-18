import Foundation
import Security
import CryptoKit

// Audits and isolated fixture setup must fail rather than request passwords.
guard SecKeychainSetUserInteractionAllowed(false) == errSecSuccess else { exit(2) }

func emit(_ value: Any) {
    let data = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    print(String(data: data, encoding: .utf8)!)
}
func check(_ status: OSStatus) throws {
    if status != errSecSuccess { throw NSError(domain: NSOSStatusErrorDomain, code: Int(status)) }
}
func signature(_ path: String) throws -> [String: Any] {
    var code: SecStaticCode?
    try check(SecStaticCodeCreateWithPath(URL(fileURLWithPath: path) as CFURL, SecCSFlags(), &code))
    var info: CFDictionary?
    try check(SecCodeCopySigningInformation(code!, SecCSFlags(rawValue: UInt32(kSecCSSigningInformation)), &info))
    let values = info! as NSDictionary
    var row: [String: Any] = ["path": path, "identifier": values[kSecCodeInfoIdentifier] ?? ""]
    if let hash = values[kSecCodeInfoUnique] as? Data { row["cdhash"] = hash.map { String(format: "%02x", $0) }.joined() }
    if let certs = values[kSecCodeInfoCertificates] as? [SecCertificate], let cert = certs.first {
        row["certificate_sha1"] = Insecure.SHA1.hash(data: SecCertificateCopyData(cert) as Data).map { String(format: "%02X", $0) }.joined()
    }
    var requirement: SecRequirement?
    if SecCodeCopyDesignatedRequirement(code!, SecCSFlags(), &requirement) == errSecSuccess, let requirement = requirement {
        var text: CFString?
        try check(SecRequirementCopyString(requirement, SecCSFlags(), &text))
        row["requirement"] = text! as String
    }
    row["valid"] = SecStaticCodeCheckValidity(code!, SecCSFlags(rawValue: UInt32(kSecCSStrictValidate)), nil) == errSecSuccess
    return row
}
do {
    let args = Array(CommandLine.arguments.dropFirst())
    if args.count == 2 && ["--create-fixture-keychain", "--delete-fixture-keychain"].contains(args[0]) {
        let path = URL(fileURLWithPath: args[1])
        guard path.lastPathComponent == "fixture.keychain-db",
              FileManager.default.fileExists(atPath: path.deletingLastPathComponent().appendingPathComponent(".mcplink-keychain-test-fixture").path) else { throw NSError(domain: "Fixture", code: 1) }
        if args[0] == "--create-fixture-keychain" {
            let raw = FileHandle.standardInput.readDataToEndOfFile()
            guard let object = try JSONSerialization.jsonObject(with: raw) as? [String: String],
                  let password = object["password"] else { throw NSError(domain: "Fixture", code: 2) }
            var before: CFArray?
            try check(SecKeychainCopySearchList(&before))
            var keychain: SecKeychain?
            let data = Data(password.utf8)
            try check(data.withUnsafeBytes { SecKeychainCreate(path.path, UInt32(data.count), $0.baseAddress, false, nil, &keychain) })
            // Test creation must not change the user's lookup/default Keychain.
            var current: CFArray?
            try check(SecKeychainCopySearchList(&current))
            let original = before as? [SecKeychain] ?? []
            let filtered = (current as? [SecKeychain] ?? []).filter { entry in
                original.contains(where: { CFEqual($0, entry) }) || !CFEqual(entry, keychain!)
            }
            try check(SecKeychainSetSearchList(filtered as CFArray))
            emit(["created": true, "test_fixture_only": true])
        } else {
            var keychain: SecKeychain?
            try check(SecKeychainOpen(path.path, &keychain))
            try check(SecKeychainDelete(keychain!))
            emit(["deleted": true, "test_fixture_only": true])
        }
    } else if args == ["--import-identity"] || (args.count == 3 && args[0] == "--import-identity" && args[1] == "--fixture-keychain") {
        // Only a newly generated signing identity enters this path. No existing
        // service credential or Keychain ACL is read, replaced or relaxed.
        let raw = FileHandle.standardInput.readDataToEndOfFile()
        guard raw.count <= 1024 * 1024,
              let value = try JSONSerialization.jsonObject(with: raw) as? [String: String],
              let blob = value["pkcs12"].flatMap({ Data(base64Encoded: $0) }),
              let password = value["password"] else { throw NSError(domain: "Input", code: 1) }
        var keychain: SecKeychain?
        if args.count == 3 {
            let path = URL(fileURLWithPath: args[2])
            guard path.lastPathComponent == "fixture.keychain-db",
                  FileManager.default.fileExists(atPath: path.deletingLastPathComponent().appendingPathComponent(".mcplink-keychain-test-fixture").path) else { throw NSError(domain: "Fixture", code: 3) }
            try check(SecKeychainOpen(path.path, &keychain))
        } else {
            try check(SecKeychainCopyDefault(&keychain))
        }
        var codesign: SecTrustedApplication?
        try check(SecTrustedApplicationCreateFromPath("/usr/bin/codesign", &codesign))
        var access: SecAccess?
        try check(SecAccessCreate("mcplink local code signing" as CFString, [codesign!] as CFArray, &access))
        let passphrase = password as CFString
        let usage = [kSecAttrCanSign] as CFArray
        let attributes = [kSecAttrIsPermanent, kSecAttrIsSensitive] as CFArray
        var params = SecItemImportExportKeyParameters()
        params.version = UInt32(SEC_KEY_IMPORT_EXPORT_PARAMS_VERSION)
        params.passphrase = Unmanaged.passUnretained(passphrase)
        params.accessRef = Unmanaged.passUnretained(access!)
        params.keyUsage = Unmanaged.passUnretained(usage)
        params.keyAttributes = Unmanaged.passUnretained(attributes)
        var format = SecExternalFormat.formatPKCS12
        var kind = SecExternalItemType.itemTypeAggregate
        var imported: CFArray?
        let status = withExtendedLifetime((passphrase, usage, attributes, access)) {
            SecItemImport(blob as CFData, "p12" as CFString, &format, &kind, SecItemImportExportFlags(), &params, keychain, &imported)
        }
        try check(status)
        emit(["imported": true, "system_trust_modified": false, "service_credentials_modified": false])
    } else {
        guard !args.isEmpty else { throw NSError(domain: "Input", code: 2) }
        let paths = Set(args.map { URL(fileURLWithPath: $0).standardizedFileURL.resolvingSymlinksInPath().path })
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/ps")
        process.arguments = ["-axo", "pid=,comm="]
        let pipe = Pipe(); process.standardOutput = pipe
        try process.run()
        let data = pipe.fileHandleForReading.readDataToEndOfFile(); process.waitUntilExit()
        var rows = [[String: Any]]()
        for line in String(data: data, encoding: .utf8)!.split(separator: "\n") {
            let parts = line.trimmingCharacters(in: .whitespaces).split(maxSplits: 1, whereSeparator: { $0.isWhitespace })
            guard parts.count == 2, let pid = Int32(parts[0]), paths.contains(String(parts[1])) else { continue }
            var code: SecCode?
            let found = SecCodeCopyGuestWithAttributes(nil, [kSecGuestAttributePid as String: NSNumber(value: pid)] as CFDictionary, SecCSFlags(), &code)
            let status = code.map { SecCodeCheckValidity($0, SecCSFlags(), nil) } ?? found
            rows.append(["pid": pid, "path": String(parts[1]), "status": status, "valid": status == errSecSuccess,
                         "message": SecCopyErrorMessageString(status, nil) as String? ?? ""])
        }
        emit(["signatures": try paths.sorted().map(signature), "processes": rows])
    }
} catch {
    // Error codes only: import input contains a private key and must never be echoed.
    emit(["error": "macOS identity operation failed", "status": (error as NSError).code])
    exit(1)
}
