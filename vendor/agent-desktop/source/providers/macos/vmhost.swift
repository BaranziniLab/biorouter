import AppKit
import Darwin
import Foundation
@preconcurrency import Virtualization

struct VMError: Error {
    let message: String
    let suggestion: String
}

struct Bundle {
    let root: URL

    init(_ path: String) {
        root = URL(fileURLWithPath: path, isDirectory: true)
    }

    var sandboxName: String { root.deletingLastPathComponent().lastPathComponent }
    var disk: URL { root.appendingPathComponent("Disk.img") }
    var auxiliaryStorage: URL { root.appendingPathComponent("AuxiliaryStorage") }
    var hardwareModel: URL { root.appendingPathComponent("HardwareModel") }
    var machineIdentifier: URL { root.appendingPathComponent("MachineIdentifier") }
    var settings: URL { root.appendingPathComponent("config.json") }
    var share: URL { root.appendingPathComponent("share", isDirectory: true) }
    var pidFile: URL { root.appendingPathComponent("vmhost.pid") }
    var startedFile: URL { root.appendingPathComponent("vmhost.started") }
    var lockFile: URL { root.appendingPathComponent("vmhost.lock") }
    var installLog: URL { root.appendingPathComponent("install.log") }
}

struct VMConfig: Codable {
    var cpuCount: Int
    var memorySize: UInt64
    var macAddress: String
}

enum Usage {
    static var noCommand: VMError {
        VMError(message: "vmhost needs one of the commands install, run, clone or ip.",
                suggestion: "Run `agent-desktop sandbox create <name>` so the provider calls vmhost itself.")
    }

    static func unknown(_ name: String) -> VMError {
        VMError(message: "vmhost does not know the command \(name).",
                suggestion: "Run `agent-desktop sandbox create <name>` so the provider calls vmhost itself.")
    }

    static var installArguments: VMError {
        VMError(message: "vmhost install needs a bundle path and an image path.",
                suggestion: "Run `agent-desktop sandbox create <name> --set ipsw=<path>` so the provider calls it correctly.")
    }

    static var runArguments: VMError {
        VMError(message: "vmhost run takes one bundle path and an optional --window with an optional title.",
                suggestion: "Run `agent-desktop sandbox start <name>` so the provider calls it correctly.")
    }

    static var cloneArguments: VMError {
        VMError(message: "vmhost clone needs a source bundle path and a destination bundle path.",
                suggestion: "Run `agent-desktop sandbox create <name> --set from=<source>` so the provider calls it correctly.")
    }

    static var ipArguments: VMError {
        VMError(message: "vmhost ip takes one bundle path.",
                suggestion: "Run `agent-desktop sandbox status <name>` so the provider calls it correctly.")
    }

    static var unwritableReply: VMError {
        VMError(message: "vmhost cannot write its JSON reply.",
                suggestion: "Run the command again, and report the failure if it repeats.")
    }
}

enum Output {
    static func succeed(_ fields: [String: Any]) {
        guard let line = encode(fields) else { fail(Usage.unwritableReply) }
        FileHandle.standardOutput.write(Data((line + "\n").utf8))
    }

    static func fail(_ error: VMError) -> Never {
        if let line = encode(["ok": false, "message": error.message, "suggestion": error.suggestion]) {
            FileHandle.standardOutput.write(Data((line + "\n").utf8))
        }
        exit(1)
    }

    static func encode(_ fields: [String: Any]) -> String? {
        guard let data = try? JSONSerialization.data(withJSONObject: fields, options: [.sortedKeys]) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

enum MACAddress {
    static func octets(_ text: String) -> [Int]? {
        let address = text.split(separator: ",").last.map(String.init) ?? text
        let parts = address.split(separator: ":")
        guard parts.count == 6 else { return nil }
        var values: [Int] = []
        for part in parts {
            guard let value = Int(part, radix: 16), (0...255).contains(value) else { return nil }
            values.append(value)
        }
        return values
    }
}

enum VersionText {
    static func macOS(_ version: OperatingSystemVersion) -> String {
        "macOS \(version.majorVersion).\(version.minorVersion).\(version.patchVersion)"
    }

    static var host: String { macOS(ProcessInfo.processInfo.operatingSystemVersion) }
}

enum PidFile {
    static func liveProcess(in bundle: Bundle) -> Int32? {
        guard let text = try? String(contentsOf: bundle.pidFile, encoding: .utf8) else { return nil }
        guard let pid = Int32(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 1 else { return nil }
        return isAlive(pid) ? pid : nil
    }

    static func isAlive(_ pid: Int32) -> Bool {
        if kill(pid, 0) == 0 { return true }
        return errno == EPERM
    }

    static func record(at url: URL) throws {
        do {
            try Data("\(getpid())\n".utf8).write(to: url, options: .atomic)
        } catch {
            throw VMError(message: "vmhost cannot write \(url.lastPathComponent).",
                          suggestion: "Check that the sandbox directory is writable, then run `agent-desktop sandbox start <name>` again.")
        }
    }

    static func clear(at url: URL) {
        try? FileManager.default.removeItem(at: url)
    }
}

enum RunLock {
    private static var descriptor: Int32 = -1

    static func acquire(_ bundle: Bundle) throws {
        let fd = open(bundle.lockFile.path, O_CREAT | O_RDWR, S_IRUSR | S_IWUSR)
        guard fd >= 0 else {
            throw VMError(message: "vmhost cannot open \(bundle.lockFile.lastPathComponent).",
                          suggestion: "Check that the sandbox directory is writable, then run `agent-desktop sandbox start \(bundle.sandboxName)` again.")
        }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
            close(fd)
            throw VMError(message: "The sandbox \(bundle.sandboxName) is already running.",
                          suggestion: "Run `agent-desktop sandbox stop \(bundle.sandboxName)`, then start it again.")
        }
        descriptor = fd
    }
}

enum BundleData {
    static func missing(_ description: String, in bundle: Bundle) -> VMError {
        VMError(message: "The sandbox has no \(description).",
                suggestion: "Run `agent-desktop sandbox create \(bundle.sandboxName)` to build a new one.")
    }

    static func invalid(_ description: String, in bundle: Bundle) -> VMError {
        VMError(message: "The sandbox \(description) is not valid.",
                suggestion: "Run `agent-desktop sandbox create \(bundle.sandboxName)` to build a new one.")
    }

    static func config(_ bundle: Bundle) throws -> VMConfig {
        let data: Data
        do {
            data = try Data(contentsOf: bundle.settings)
        } catch {
            throw missing("config.json", in: bundle)
        }
        guard let config = try? JSONDecoder().decode(VMConfig.self, from: data),
              config.cpuCount > 0,
              config.memorySize > 0,
              MACAddress.octets(config.macAddress) != nil else {
            throw invalid("config.json", in: bundle)
        }
        return config
    }

    static func write(_ config: VMConfig, to bundle: Bundle) throws {
        do {
            try JSONEncoder().encode(config).write(to: bundle.settings, options: .atomic)
        } catch {
            throw VMError(message: "vmhost cannot write config.json.",
                          suggestion: "Check that the disk has free space, then run the command again.")
        }
    }

    static func hardwareModel(_ bundle: Bundle) throws -> VZMacHardwareModel {
        let data: Data
        do {
            data = try Data(contentsOf: bundle.hardwareModel)
        } catch {
            throw missing("HardwareModel file", in: bundle)
        }
        guard let model = VZMacHardwareModel(dataRepresentation: data) else {
            throw invalid("HardwareModel file", in: bundle)
        }
        return model
    }

    static func machineIdentifier(_ bundle: Bundle) throws -> VZMacMachineIdentifier {
        let data: Data
        do {
            data = try Data(contentsOf: bundle.machineIdentifier)
        } catch {
            throw missing("MachineIdentifier file", in: bundle)
        }
        guard let identifier = VZMacMachineIdentifier(dataRepresentation: data) else {
            throw invalid("MachineIdentifier file", in: bundle)
        }
        return identifier
    }

    static func require(_ url: URL, in bundle: Bundle) throws {
        guard FileManager.default.fileExists(atPath: url.path) else {
            throw missing("\(url.lastPathComponent) file", in: bundle)
        }
    }
}

struct VMInputs {
    let bundle: Bundle
    let config: VMConfig
    let hardwareModel: VZMacHardwareModel
    let machineIdentifier: VZMacMachineIdentifier
    let auxiliaryStorage: VZMacAuxiliaryStorage
}

enum VMBuilder {
    static func configuration(_ inputs: VMInputs) throws -> VZVirtualMachineConfiguration {
        guard inputs.hardwareModel.isSupported else {
            throw VMError(message: "This Mac cannot boot the hardware model saved in the sandbox.",
                          suggestion: "Run `agent-desktop sandbox create \(inputs.bundle.sandboxName)` on this Mac to build a new one.")
        }
        guard let address = VZMACAddress(string: inputs.config.macAddress) else {
            throw VMError(message: "The sandbox config.json holds an invalid MAC address.",
                          suggestion: "Run `agent-desktop sandbox create \(inputs.bundle.sandboxName)` to build a new one.")
        }

        let platform = VZMacPlatformConfiguration()
        platform.hardwareModel = inputs.hardwareModel
        platform.machineIdentifier = inputs.machineIdentifier
        platform.auxiliaryStorage = inputs.auxiliaryStorage

        let disk: VZDiskImageStorageDeviceAttachment
        do {
            disk = try VZDiskImageStorageDeviceAttachment(url: inputs.bundle.disk, readOnly: false)
        } catch {
            throw VMError(message: "The sandbox disk image cannot be opened.",
                          suggestion: "Check that Disk.img exists and that the disk has free space, then run the command again.")
        }

        let network = VZVirtioNetworkDeviceConfiguration()
        network.macAddress = address
        network.attachment = VZNATNetworkDeviceAttachment()

        let share = VZVirtioFileSystemDeviceConfiguration(tag: "com.apple.virtio-fs.automount")
        share.share = VZSingleDirectoryShare(directory: VZSharedDirectory(url: inputs.bundle.share, readOnly: true))

        let graphics = VZMacGraphicsDeviceConfiguration()
        graphics.displays = [VZMacGraphicsDisplayConfiguration(widthInPixels: 1920, heightInPixels: 1200, pixelsPerInch: 144)]

        let configuration = VZVirtualMachineConfiguration()
        configuration.bootLoader = VZMacOSBootLoader()
        configuration.platform = platform
        configuration.cpuCount = inputs.config.cpuCount
        configuration.memorySize = inputs.config.memorySize
        configuration.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: disk)]
        configuration.networkDevices = [network]
        configuration.directorySharingDevices = [share]
        configuration.graphicsDevices = [graphics]
        configuration.keyboards = [VZUSBKeyboardConfiguration()]
        configuration.pointingDevices = [VZUSBScreenCoordinatePointingDeviceConfiguration()]

        do {
            try configuration.validate()
        } catch {
            throw VMError(message: "The saved virtual machine settings are not valid on this Mac.",
                          suggestion: "Run `agent-desktop sandbox create \(inputs.bundle.sandboxName)` on this Mac to build a new one.")
        }
        return configuration
    }
}

enum VMErrors {
    static func machine(_ error: Error, bundle: Bundle) -> VMError {
        let failure = error as NSError
        if failure.domain == VZErrorDomain, failure.code == VZError.Code.virtualMachineLimitExceeded.rawValue {
            return VMError(message: "macOS allows only two macOS VMs running at once.",
                           suggestion: "Stop another macOS sandbox, then start this one again.")
        }
        return VMError(message: "The virtual machine failed: \(failure.localizedDescription)",
                       suggestion: "Run `agent-desktop sandbox status \(bundle.sandboxName)` and read the vm/vmhost.log file for the reason.")
    }

    static func posix(_ message: String) -> VMError {
        VMError(message: "\(message): \(String(cString: strerror(errno))).",
                suggestion: "Check that the sandbox directory is writable and has free space, then run the command again.")
    }
}

final class ProgressLog: @unchecked Sendable {
    private let handle: FileHandle
    private let lock = NSLock()
    private var lastPercent = -1

    private init(handle: FileHandle) {
        self.handle = handle
    }

    static func create(at url: URL) throws -> ProgressLog {
        guard FileManager.default.fileExists(atPath: url.path) || FileManager.default.createFile(atPath: url.path, contents: nil) else {
            throw VMError(message: "vmhost cannot create \(url.lastPathComponent).",
                          suggestion: "Check that the sandbox directory is writable, then run `agent-desktop sandbox create` again.")
        }
        do {
            let handle = try FileHandle(forWritingTo: url)
            handle.seekToEndOfFile()
            return ProgressLog(handle: handle)
        } catch {
            throw VMError(message: "vmhost cannot open \(url.lastPathComponent).",
                          suggestion: "Check that the sandbox directory is writable, then run `agent-desktop sandbox create` again.")
        }
    }

    func record(_ fraction: Double) {
        let percent = min(100, max(0, Int((fraction * 100).rounded(.down))))
        lock.withLock {
            guard percent > lastPercent else { return }
            lastPercent = percent
            try? handle.write(contentsOf: Data("\(percent)%\n".utf8))
        }
    }
}

struct InstallCommand {
    static func execute(_ arguments: [String]) throws {
        guard arguments.count == 2 else { throw Usage.installArguments }
        let bundle = Bundle(arguments[0])
        let imageURL = URL(fileURLWithPath: arguments[1], isDirectory: false)
        guard FileManager.default.isReadableFile(atPath: imageURL.path) else {
            throw VMError(message: "The image file \(imageURL.path) cannot be read.",
                          suggestion: "Download the image again, or pass the path of an image that exists with `--set ipsw=<path>`.")
        }
        let box = ResultBox()
        Task {
            do {
                try await perform(imageURL, bundle)
                box.finish(.success(()))
            } catch {
                box.finish(.failure(error))
            }
        }
        MainLoop.pump(until: { box.value() != nil })
        guard let result = box.value() else {
            throw VMError(message: "The image install stopped without reporting a result.",
                          suggestion: "Run `agent-desktop sandbox create \(bundle.sandboxName)` again and read the vm/install.log file.")
        }
        try result.get()
        Output.succeed(["ok": true, "status": "installed"])
    }

    @MainActor
    static func perform(_ imageURL: URL, _ bundle: Bundle) async throws {
        let image: VZMacOSRestoreImage
        do {
            image = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<VZMacOSRestoreImage, Error>) in
                VZMacOSRestoreImage.load(from: imageURL) { result in
                    continuation.resume(with: result)
                }
            }
        } catch {
            throw VMError(message: "The image file \(imageURL.lastPathComponent) is not a macOS install image that this Mac can read.",
                          suggestion: "Download the image again, then run `agent-desktop sandbox create \(bundle.sandboxName) --set ipsw=<path>`.")
        }
        guard image.isSupported else { throw unsupported(image, bundle: bundle) }
        guard let requirements = image.mostFeaturefulSupportedConfiguration else {
            throw VMError(message: "The image \(VersionText.macOS(image.operatingSystemVersion)) reports no configuration that this Mac running \(VersionText.host) can boot.",
                          suggestion: "Download the newest macOS image Apple offers for this Mac, then run `agent-desktop sandbox create \(bundle.sandboxName) --set ipsw=<path>`.")
        }

        try prepare(bundle)
        let config = VMConfig(
            cpuCount: max(4, requirements.minimumSupportedCPUCount),
            memorySize: max(UInt64(8) * 1024 * 1024 * 1024, requirements.minimumSupportedMemorySize),
            macAddress: VZMACAddress.randomLocallyAdministered().string
        )
        let hardwareModel = requirements.hardwareModel
        let machineIdentifier = VZMacMachineIdentifier()
        try hardwareModel.dataRepresentation.write(to: bundle.hardwareModel, options: .atomic)
        try machineIdentifier.dataRepresentation.write(to: bundle.machineIdentifier, options: .atomic)
        try BundleData.write(config, to: bundle)
        try SparseDisk.create(at: bundle.disk)
        let log = try ProgressLog.create(at: bundle.installLog)

        let auxiliaryStorage: VZMacAuxiliaryStorage
        do {
            auxiliaryStorage = try VZMacAuxiliaryStorage(creatingStorageAt: bundle.auxiliaryStorage, hardwareModel: hardwareModel, options: [])
        } catch {
            throw VMError(message: "vmhost cannot create the guest's auxiliary storage.",
                          suggestion: "Check that the sandbox directory is writable and has free space, then run `agent-desktop sandbox create` again.")
        }
        let configuration = try VMBuilder.configuration(VMInputs(bundle: bundle, config: config, hardwareModel: hardwareModel, machineIdentifier: machineIdentifier, auxiliaryStorage: auxiliaryStorage))
        let installer = VZMacOSInstaller(virtualMachine: VZVirtualMachine(configuration: configuration), restoringFromImageAt: imageURL)
        let observation = installer.progress.observe(\.fractionCompleted, options: [.initial, .new]) { progress, _ in
            log.record(progress.fractionCompleted)
        }
        do {
            try await installer.install()
        } catch {
            withExtendedLifetime(observation) {}
            throw VMError(message: "The image \(VersionText.macOS(image.operatingSystemVersion)) did not finish installing: \(error.localizedDescription)",
                          suggestion: "Run `agent-desktop sandbox create \(bundle.sandboxName)` again and read the vm/install.log file if it fails again.")
        }
        withExtendedLifetime(observation) {}
    }

    static func unsupported(_ image: VZMacOSRestoreImage, bundle: Bundle) -> VMError {
        VMError(message: "The image \(VersionText.macOS(image.operatingSystemVersion)) does not support this Mac running \(VersionText.host).",
                suggestion: "Download the newest macOS image Apple offers for this Mac, then run `agent-desktop sandbox create \(bundle.sandboxName) --set ipsw=<path>`.")
    }

    static func prepare(_ bundle: Bundle) throws {
        let bookkeeping: Set<String> = ["install.log", "install.pid", "share", "vmhost.lock"]
        if FileManager.default.fileExists(atPath: bundle.root.path) {
            let contents = (try? FileManager.default.contentsOfDirectory(atPath: bundle.root.path)) ?? []
            guard contents.allSatisfy({ bookkeeping.contains($0) || $0.hasPrefix(".") }) else {
                throw VMError(message: "The bundle directory \(bundle.root.path) is not empty.",
                              suggestion: "Remove it or choose an empty directory, then run `agent-desktop sandbox create` again.")
            }
        } else {
            do {
                try FileManager.default.createDirectory(at: bundle.root, withIntermediateDirectories: true)
            } catch {
                throw VMError(message: "vmhost cannot create the bundle directory \(bundle.root.path).",
                              suggestion: "Check that the parent directory is writable, then run `agent-desktop sandbox create` again.")
            }
        }
        guard !FileManager.default.fileExists(atPath: bundle.share.path) else { return }
        do {
            try FileManager.default.createDirectory(at: bundle.share, withIntermediateDirectories: false)
        } catch {
            throw VMError(message: "vmhost cannot create the sandbox share folder.",
                          suggestion: "Check that the sandbox directory is writable, then run `agent-desktop sandbox create` again.")
        }
    }
}

enum SparseDisk {
    static func create(at url: URL) throws {
        let descriptor = open(url.path, O_CREAT | O_EXCL | O_WRONLY, S_IRUSR | S_IWUSR)
        guard descriptor >= 0 else { throw VMErrors.posix("The disk image \(url.lastPathComponent) cannot be created") }
        guard ftruncate(descriptor, off_t(64) * 1024 * 1024 * 1024) == 0 else {
            let failure = VMErrors.posix("The disk image \(url.lastPathComponent) cannot be sized")
            close(descriptor)
            unlink(url.path)
            throw failure
        }
        guard close(descriptor) == 0 else { throw VMErrors.posix("The disk image \(url.lastPathComponent) cannot be written") }
    }
}

final class VMBox: @unchecked Sendable {
    let machine: VZVirtualMachine
    private let delegate: VMHostDelegate

    init(_ machine: VZVirtualMachine, delegate: VMHostDelegate) {
        self.machine = machine
        self.delegate = delegate
        machine.delegate = delegate
    }
}

final class ResultBox: @unchecked Sendable {
    private let lock = NSLock()
    private var result: Result<Void, Error>?

    @discardableResult
    func finish(_ value: Result<Void, Error>) -> Bool {
        lock.withLock {
            guard result == nil else { return false }
            result = value
            return true
        }
    }

    func value() -> Result<Void, Error>? {
        lock.withLock { result }
    }
}

enum MainLoop {
    static func pump(until finished: () -> Bool) {
        while !finished() {
            RunLoop.main.run(mode: .default, before: Date(timeIntervalSinceNow: 0.05))
        }
    }
}

enum ApplicationLoop {
    static func run() {
        NSApplication.shared.run()
    }

    static func stop() {
        NSApplication.shared.stop(nil)
        let wake = NSEvent.otherEvent(with: .applicationDefined, location: .zero, modifierFlags: [], timestamp: 0,
                                      windowNumber: 0, context: nil, subtype: 0, data1: 0, data2: 0)
        if let wake {
            NSApplication.shared.postEvent(wake, atStart: false)
        }
    }
}

final class VMHostDelegate: NSObject, VZVirtualMachineDelegate, @unchecked Sendable {
    private let finish: (Result<Void, Error>) -> Void

    init(_ finish: @escaping (Result<Void, Error>) -> Void) {
        self.finish = finish
    }

    func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        finish(.success(()))
    }

    func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        finish(.failure(error))
    }
}

final class WindowCloseDelegate: NSObject, NSWindowDelegate {
    var onClose: () -> Void = {}

    func windowWillClose(_ notification: Notification) {
        onClose()
    }
}

struct SandboxWindow {
    let window: NSWindow
    let closeDelegate = WindowCloseDelegate()
    static var title: String?

    static func make(bundle: Bundle, machine: VZVirtualMachine) -> SandboxWindow {
        let application = NSApplication.shared
        application.setActivationPolicy(.regular)
        application.finishLaunching()
        let frame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let window = NSWindow(contentRect: frame, styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        window.title = title ?? bundle.sandboxName
        let view = VZVirtualMachineView(frame: frame)
        view.autoresizingMask = [.width, .height]
        view.virtualMachine = machine
        window.contentView = view
        window.center()
        window.makeKeyAndOrderFront(nil)
        application.activate(ignoringOtherApps: true)
        let sandboxWindow = SandboxWindow(window: window)
        window.delegate = sandboxWindow.closeDelegate
        return sandboxWindow
    }

    func close() {
        window.close()
    }
}

struct RunCommand {
    static func execute(_ arguments: [String]) throws {
        guard arguments.count == 1 || ((2...3).contains(arguments.count) && arguments[1] == "--window") else {
            throw Usage.runArguments
        }
        let bundle = Bundle(arguments[0])
        let showWindow = arguments.count >= 2
        SandboxWindow.title = arguments.count == 3 ? arguments[2] : nil

        try RunLock.acquire(bundle)
        if let pid = PidFile.liveProcess(in: bundle) {
            throw VMError(message: "The sandbox \(bundle.sandboxName) is already running with pid \(pid).",
                          suggestion: "Run `agent-desktop sandbox stop \(bundle.sandboxName)`, then start it again.")
        }
        let config = try BundleData.config(bundle)
        let hardwareModel = try BundleData.hardwareModel(bundle)
        let machineIdentifier = try BundleData.machineIdentifier(bundle)
        try BundleData.require(bundle.disk, in: bundle)
        try BundleData.require(bundle.auxiliaryStorage, in: bundle)
        do {
            try FileManager.default.createDirectory(at: bundle.share, withIntermediateDirectories: true)
        } catch {
            throw VMError(message: "vmhost cannot create the sandbox share folder.",
                          suggestion: "Check that the sandbox directory is writable, then run `agent-desktop sandbox start \(bundle.sandboxName)` again.")
        }

        let inputs = VMInputs(bundle: bundle, config: config, hardwareModel: hardwareModel, machineIdentifier: machineIdentifier, auxiliaryStorage: VZMacAuxiliaryStorage(url: bundle.auxiliaryStorage))
        let configuration = try VMBuilder.configuration(inputs)
        try PidFile.record(at: bundle.pidFile)
        defer {
            PidFile.clear(at: bundle.pidFile)
            PidFile.clear(at: bundle.startedFile)
        }
        try boot(bundle: bundle, configuration: configuration, showWindow: showWindow)
    }

    static func boot(bundle: Bundle, configuration: VZVirtualMachineConfiguration, showWindow: Bool) throws {
        let queue = showWindow ? DispatchQueue.main : DispatchQueue(label: "com.agent-desktop.vmhost")
        let machine = VZVirtualMachine(configuration: configuration, queue: queue)
        let outcome = ResultBox()
        let window = showWindow ? SandboxWindow.make(bundle: bundle, machine: machine) : nil
        let finish: (Result<Void, Error>) -> Void = { result in
            guard outcome.finish(result) else { return }
            if showWindow {
                ApplicationLoop.stop()
            }
            window?.close()
        }
        let box = VMBox(machine, delegate: VMHostDelegate(finish))

        signal(SIGTERM, SIG_IGN)
        let termination = DispatchSource.makeSignalSource(signal: SIGTERM, queue: queue)
        var stopping = false
        let requestStop = {
            guard !stopping, outcome.value() == nil else { return }
            stopping = true
            if box.machine.canRequestStop {
                do {
                    try box.machine.requestStop()
                } catch {
                    FileHandle.standardError.write(Data("vmhost: graceful stop request failed: \(error.localizedDescription)\n".utf8))
                }
            }
            queue.asyncAfter(deadline: .now() + 15) {
                guard outcome.value() == nil else { return }
                box.machine.stop { error in
                    if let error {
                        finish(.failure(error))
                    } else {
                        finish(.success(()))
                    }
                }
            }
        }
        termination.setEventHandler(handler: requestStop)
        termination.resume()
        window?.closeDelegate.onClose = requestStop

        queue.async {
            box.machine.start { result in
                if case let .failure(error) = result {
                    finish(.failure(error))
                } else {
                    FileManager.default.createFile(atPath: bundle.startedFile.path, contents: nil)
                }
            }
        }

        if showWindow {
            ApplicationLoop.run()
        } else {
            MainLoop.pump(until: { outcome.value() != nil })
        }
        termination.cancel()
        window?.close()

        guard let result = outcome.value() else {
            throw VMError(message: "The virtual machine stopped without reporting a result.",
                          suggestion: "Run `agent-desktop sandbox status \(bundle.sandboxName)` and read the vm/vmhost.log file.")
        }
        if case let .failure(error) = result {
            throw VMErrors.machine(error, bundle: bundle)
        }
        Output.succeed(["ok": true, "status": "stopped"])
    }
}

struct CloneCommand {
    static func execute(_ arguments: [String]) throws {
        guard arguments.count == 2 else { throw Usage.cloneArguments }
        let source = Bundle(arguments[0])
        let destination = Bundle(arguments[1])

        guard FileManager.default.fileExists(atPath: source.root.path) else {
            throw VMError(message: "The source bundle does not exist.",
                          suggestion: "Run `agent-desktop sandbox status <source>` to find a sandbox that still exists, then create this one again.")
        }
        guard !FileManager.default.fileExists(atPath: destination.root.path) else {
            throw VMError(message: "The destination bundle \(destination.root.path) already exists.",
                          suggestion: "Remove it, then run `agent-desktop sandbox create` again.")
        }
        if let pid = PidFile.liveProcess(in: source) {
            throw VMError(message: "The source sandbox \(source.sandboxName) is running with pid \(pid).",
                          suggestion: "Stop \(source.sandboxName) first with `agent-desktop sandbox stop \(source.sandboxName)`, then create this sandbox again.")
        }
        var cloned = try BundleData.config(source)
        try BundleData.require(source.disk, in: source)
        try BundleData.require(source.auxiliaryStorage, in: source)
        try BundleData.require(source.hardwareModel, in: source)

        cloned.macAddress = VZMACAddress.randomLocallyAdministered().string
        let staging = destination.root.deletingLastPathComponent().appendingPathComponent(".vmhost-clone-\(UUID().uuidString)", isDirectory: true)
        let staged = Bundle(staging.path)
        do {
            try FileManager.default.createDirectory(at: staging, withIntermediateDirectories: false)
            try FileManager.default.createDirectory(at: staged.share, withIntermediateDirectories: false)
            try cloneFile(source.disk, staged.disk)
            try cloneFile(source.auxiliaryStorage, staged.auxiliaryStorage)
            try FileManager.default.copyItem(at: source.hardwareModel, to: staged.hardwareModel)
            try VZMacMachineIdentifier().dataRepresentation.write(to: staged.machineIdentifier, options: .atomic)
            try BundleData.write(cloned, to: staged)
            try FileManager.default.moveItem(at: staging, to: destination.root)
        } catch {
            try? FileManager.default.removeItem(at: staging)
            if let error = error as? VMError { throw error }
            throw VMErrors.posix("The sandbox disk could not be cloned")
        }
        Output.succeed(["ok": true, "status": "cloned"])
    }

    static func cloneFile(_ source: URL, _ destination: URL) throws {
        let status = source.path.withCString { sourcePath in
            destination.path.withCString { destinationPath in
                clonefile(sourcePath, destinationPath, 0)
            }
        }
        guard status == 0 else { throw VMErrors.posix("The sandbox file \(source.lastPathComponent) could not be cloned") }
    }
}

enum LeaseTable {
    static func newestAddress(for octets: [Int], in contents: String) -> String? {
        var leases: [(mine: Bool, address: String, expiry: UInt64)] = []
        var record: [String: String] = [:]
        for line in contents.split(whereSeparator: \.isNewline) {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed == "{" {
                record = [:]
            } else if trimmed == "}" {
                if let hardware = record["hw_address"], let address = record["ip_address"] {
                    let digits = record["lease"]?.dropFirst(2) ?? ""
                    leases.append((MACAddress.octets(hardware) == octets, address, UInt64(digits, radix: 16) ?? 0))
                }
            } else if let separator = trimmed.firstIndex(of: "=") {
                record[String(trimmed[..<separator])] = String(trimmed[trimmed.index(after: separator)...])
            }
        }
        guard let newest = leases.filter(\.mine).max(by: { $0.expiry < $1.expiry }) else { return nil }
        let reassigned = leases.contains { !$0.mine && $0.address == newest.address && $0.expiry > newest.expiry }
        return reassigned ? nil : newest.address
    }
}

struct IpCommand {
    static func execute(_ arguments: [String]) throws {
        guard arguments.count == 1 else { throw Usage.ipArguments }
        let bundle = Bundle(arguments[0])
        let config = try BundleData.config(bundle)
        let octets = MACAddress.octets(config.macAddress)!
        let contents: String
        do {
            contents = try String(contentsOfFile: "/var/db/dhcpd_leases", encoding: .utf8)
        } catch {
            throw VMError(message: "The DHCP lease file /var/db/dhcpd_leases cannot be read.",
                          suggestion: "Run `agent-desktop sandbox status \(bundle.sandboxName)` to see whether the sandbox is running.")
        }
        guard let address = LeaseTable.newestAddress(for: octets, in: contents) else {
            throw VMError(message: "The VM has no network address yet.",
                          suggestion: "Wait a few seconds, then run `agent-desktop sandbox status \(bundle.sandboxName)`.")
        }
        Output.succeed(["ok": true, "ip": address])
    }
}

/// Leaves the caller's POSIX session so a launcher that exits or is killed does not take a running sandbox
/// host down with it. A process-group leader may not start a session, so one first joins its parent's group, or its session leader's when the parent is gone.
enum CallerSession {
    static func leave() {
        if getsid(0) == getpid() || setsid() != -1 { return }
        for group in [getpgid(getppid()), getsid(0)] {
            if setpgid(0, group) == 0, setsid() != -1 { return }
        }
        let reason = String(cString: strerror(errno))
        FileHandle.standardError.write(Data("vmhost: could not leave the caller's session, so the sandbox stops with its launcher: \(reason)\n".utf8))
    }
}

enum VMHost {
    static func run(_ arguments: [String]) -> Int32 {
        do {
            guard let command = arguments.first else { throw Usage.noCommand }
            let rest = Array(arguments.dropFirst())
            switch command {
            case "install":
                try InstallCommand.execute(rest)
            case "run":
                CallerSession.leave()
                try RunCommand.execute(rest)
            case "clone":
                try CloneCommand.execute(rest)
            case "ip":
                try IpCommand.execute(rest)
            default:
                throw Usage.unknown(command)
            }
            return 0
        } catch let error as VMError {
            Output.fail(error)
        } catch {
            let failure = error as NSError
            Output.fail(VMError(message: "vmhost failed: \(failure.localizedDescription)",
                                suggestion: "Run the command again, and read the sandbox's vm/vmhost.log file if it fails again."))
        }
    }
}

exit(VMHost.run(Array(CommandLine.arguments.dropFirst())))
