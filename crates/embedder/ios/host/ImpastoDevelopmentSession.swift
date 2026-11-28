#if DEBUG
import Darwin
import Dispatch
import Foundation

/// Watches container copies during Wi-Fi development. Rust owns parsing and receipts.
@MainActor
final class ImpastoDevelopmentSession {
    static let current: ImpastoDevelopmentSession? = {
        let environment = ProcessInfo.processInfo.environment
        guard environment["IMPASTO_DEV_FILES"] == "1",
              let token = environment["IMPASTO_DEV_SESSION"], token.utf8.count == 64,
              token.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
              let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
        else { return nil }
        return ImpastoDevelopmentSession(
            directory: caches.appendingPathComponent("ImpastoUpdates", isDirectory: true)
                .appendingPathComponent(token, isDirectory: true)
        )
    }()

    private let directory: URL
    private weak var owner: AnyObject?
    private var submit: ((UInt64) -> Void)?
    private var watcher: DispatchSourceFileSystemObject?

    private init(directory: URL) {
        self.directory = directory
    }

    func attach(owner: AnyObject, submit: @escaping (UInt64) -> Void) {
        guard self.owner == nil else { return }
        let descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else {
            NSLog("Impasto development directory could not be watched")
            return
        }
        self.owner = owner
        self.submit = submit
        let watcher = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor, eventMask: [.write, .extend], queue: .main
        )
        watcher.setEventHandler { [weak self] in
            Task { @MainActor in self?.drainReadyRequests() }
        }
        watcher.setCancelHandler { close(descriptor) }
        self.watcher = watcher
        watcher.resume()
        drainReadyRequests()
    }

    func detach(owner: AnyObject) {
        guard self.owner === owner else { return }
        self.owner = nil
        submit = nil
        watcher?.cancel()
        watcher = nil
    }

    private func drainReadyRequests() {
        guard owner != nil,
              let files = try? FileManager.default.contentsOfDirectory(atPath: directory.path)
        else { return }
        // The CLI publishes the marker after devicectl finishes copying the frame.
        let prefix = "ready-"
        for file in files where file.hasPrefix(prefix) {
            if let revision = UInt64(file.dropFirst(prefix.count)) {
                submit?(revision)
            }
        }
    }
}
#endif
