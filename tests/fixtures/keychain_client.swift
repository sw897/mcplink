import Foundation

// This caller is signed twice with different embedded build markers by the
// acceptance script. Its stdin/stdout contain synthetic fixture values only.
let build = "MCPLINK_FIXTURE_BUILD"
let child = Process()
child.executableURL = URL(fileURLWithPath: CommandLine.arguments[1])
child.arguments = Array(CommandLine.arguments.dropFirst(2))
child.environment = [:]
let input = Pipe(), output = Pipe(), errors = Pipe()
child.standardInput = input; child.standardOutput = output; child.standardError = errors
do {
    try child.run()
    let bytes = FileHandle.standardInput.readDataToEndOfFile()
    try input.fileHandleForWriting.write(contentsOf: bytes)
    try input.fileHandleForWriting.close()
    let response = output.fileHandleForReading.readDataToEndOfFile()
    child.waitUntilExit()
    if var value = try JSONSerialization.jsonObject(with: response) as? [String: Any] {
        value["caller_build"] = build
        FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject: value))
    }
    exit(child.terminationStatus)
} catch {
    print("{\"ok\":false,\"error\":\"fixture_transport\"}")
    exit(1)
}
