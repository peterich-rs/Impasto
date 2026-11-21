#if DEBUG
import Foundation

/// Reads development updates from the app container. The CLI grants access
/// with a fresh UUID in the environment of the launched process.
@MainActor
final class ImpastoDevelopmentSession {
    static let current: ImpastoDevelopmentSession? = {
        guard let value = ProcessInfo.processInfo.environment["IMPASTO_DEV_SESSION"],
              let token = UUID(uuidString: value), token.uuidString.lowercased() == value,
              let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first
        else { return nil }
        let root = caches.appendingPathComponent("ImpastoUpdates", isDirectory: true)
        return ImpastoDevelopmentSession(token: value, root: root)
    }()

    private struct Update: Decodable, Sendable {
        let token: String
        let revision: UInt64
        let source: String
    }

    private struct Receipt: Encodable {
        let token: String
        let revision: UInt64
        let error: String?
    }

    private let token: String
    private let directory: URL
    private weak var owner: AnyObject?
    private var submit: ((UInt64, [UInt8]) -> Bool)?
    private var timer: Timer?
    private var pending: UInt64?
    private var latest: UInt64 = 0
    private var reading = false

    private init(token: String, root: URL) {
        self.token = token
        directory = root.appendingPathComponent(token, isDirectory: true)
        // A new process has a new token. Remove previous unused session
        // directories so large debug bundles do not accumulate in caches.
        if let previous = try? FileManager.default.contentsOfDirectory(at: root, includingPropertiesForKeys: nil) {
            for old in previous where old != directory && UUID(uuidString: old.lastPathComponent) != nil {
                try? FileManager.default.removeItem(at: old)
            }
        }
    }

    func attach(owner: AnyObject, submit: @escaping (UInt64, [UInt8]) -> Bool) {
        guard self.owner == nil else { return }
        do {
            try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
            try Data(token.utf8).write(to: directory.appendingPathComponent("ready"), options: .atomic)
        } catch {
            NSLog("Impasto development directory is unavailable: %@", error.localizedDescription)
            return
        }
        self.owner = owner
        self.submit = submit
        timer?.invalidate()
        let timer = Timer(timeInterval: 0.25, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.poll() }
        }
        RunLoop.main.add(timer, forMode: .common)
        self.timer = timer
    }

    func detach(owner: AnyObject) {
        guard self.owner === owner else { return }
        if let pending {
            complete(owner: owner, revision: pending, error: "The development scene closed during update")
        }
        self.owner = nil
        submit = nil
        timer?.invalidate()
        timer = nil
        try? FileManager.default.removeItem(at: directory.appendingPathComponent("ready"))
    }

    func complete(owner: AnyObject, revision: UInt64, error: String) {
        guard self.owner === owner, pending == revision else { return }
        pending = nil
        latest = revision
        let receipt = Receipt(token: token, revision: revision, error: error.isEmpty ? nil : error)
        do {
            let data = try JSONEncoder().encode(receipt)
            try data.write(to: directory.appendingPathComponent("receipt.json"), options: .atomic)
        } catch {
            NSLog("Impasto development receipt could not be written: %@", error.localizedDescription)
        }
    }

    private func poll() {
        guard let owner, pending == nil, !reading else { return }
        reading = true
        let path = directory.appendingPathComponent("update.json")
        Task { [weak self, weak owner] in
            let update = await Task.detached(priority: .utility) {
                guard let size = try? path.resourceValues(forKeys: [.fileSizeKey]).fileSize,
                      size <= 16 * 1024 * 1024,
                      let data = try? Data(contentsOf: path, options: .mappedIfSafe),
                      data.count <= 16 * 1024 * 1024
                else { return nil as Update? }
                return try? JSONDecoder().decode(Update.self, from: data)
            }.value
            guard let self else { return }
            reading = false
            guard let owner, self.owner === owner, let update,
                  update.token == token, update.revision > latest
            else { return }
            let source = Array(update.source.utf8)
            pending = update.revision
            try? FileManager.default.removeItem(at: path)
            guard source.count <= 8 * 1024 * 1024,
                  submit?(update.revision, source) == true
            else {
                complete(owner: owner, revision: update.revision, error: "Impasto rejected the update request")
                return
            }
        }
    }
}
#endif
