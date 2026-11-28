import CoreLocation
import Foundation
import ImpastoHost

private struct LocationOptions: Decodable {
    var id: UInt64?
    var accuracy: String?
    var timeoutMs: UInt32?
    var maximumAgeMs: UInt32?
    var distanceFilterMeters: Double?
    var allowBackground: Bool?

    var desired: CLLocationAccuracy {
        switch accuracy {
        case "lowest": kCLLocationAccuracyThreeKilometers
        case "low": kCLLocationAccuracyKilometer
        case "medium": kCLLocationAccuracyHundredMeters
        case "best": kCLLocationAccuracyBest
        case "bestForNavigation": kCLLocationAccuracyBestForNavigation
        default: kCLLocationAccuracyNearestTenMeters
        }
    }
}

private struct Position: Encodable {
    let latitude: Double, longitude: Double, accuracy: Double
    let altitude: Double?, altitudeAccuracy: Double?
    let heading: Double?, headingAccuracy: Double?
    let speed: Double?, speedAccuracy: Double?
    let timestampMs: Double
    let isMocked: Bool
    init(_ value: CLLocation) {
        latitude = value.coordinate.latitude
        longitude = value.coordinate.longitude
        accuracy = value.horizontalAccuracy
        altitude = value.verticalAccuracy >= 0 ? value.altitude : nil
        altitudeAccuracy = value.verticalAccuracy >= 0 ? value.verticalAccuracy : nil
        heading = value.course >= 0 ? value.course : nil
        headingAccuracy = value.courseAccuracy >= 0 ? value.courseAccuracy : nil
        speed = value.speed >= 0 ? value.speed : nil
        speedAccuracy = value.speedAccuracy >= 0 ? value.speedAccuracy : nil
        timestampMs = value.timestamp.timeIntervalSince1970 * 1000
        isMocked = value.sourceInformation?.isSimulatedBySoftware ?? false
    }
}

@MainActor
private final class Tracker: NSObject, @preconcurrency CLLocationManagerDelegate {
    let manager = CLLocationManager()
    var location: ((CLLocation) -> Void)?
    var error: ((CLError.Code) -> Void)?
    var authorization: (() -> Void)?
    init(options: LocationOptions) {
        super.init()
        manager.delegate = self
        manager.desiredAccuracy = options.desired
        manager.distanceFilter = options.distanceFilterMeters ?? 10
    }
    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        if let value = locations.last(where: { $0.horizontalAccuracy >= 0 }) { location?(value) }
    }
    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        self.error?((error as? CLError)?.code ?? .locationUnknown)
    }
    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) { authorization?() }
    func stop() { manager.stopUpdatingLocation(); manager.delegate = nil }
}

@MainActor
public final class ImpastoGeolocatorPlugin: ImpastoPlugin {
    private struct Control: Decodable { let topic: String; let params: String? }
    private struct One { let request: ImpastoNativeRequest; let tracker: Tracker; let timer: Task<Void, Never> }
    public let identifier = "@impasto/geolocator"
    private weak var host: ImpastoHost?
    private var pending: [UInt64: One] = [:]
    private var cancelled: [UInt64] = []
    private var position: Tracker?
    private var service: Tracker?
    private var serviceTimer: Timer?
    private var lastService: Bool?
    private var latest: CLLocation?
    private var background = false

    public static func register(with host: ImpastoHost) { host.registerPlugin(Self()) }
    public static func register(with host: ImpastoHost, enabled: Set<String>, registrars: [any ImpastoGeolocatorFeature.Type]) {
        let plugin = Self()
        plugin.background = enabled.contains("background")
        host.registerPlugin(plugin)
    }
    public func installed(in host: ImpastoHost) { self.host = host }

    public func handle(_ request: ImpastoNativeRequest) -> Bool {
        do {
            switch request.operation {
            case "\(identifier)/current": try startCurrent(request)
            case "\(identifier)/last":
                let value = latest ?? CLLocationManager().location
                request.resolveJSON(value.flatMap { $0.horizontalAccuracy >= 0 ? Position($0) : nil })
            case "\(identifier)/service":
                Task { @MainActor in request.resolveJSON(await Self.serviceEnabled()) }
            case "\(identifier)/cancel":
                let id = try request.decodeJSON(UInt64.self)
                if pending[id] != nil { fail(id, "cancelled", "Location request cancelled") }
                else {
                    cancelled.append(id)
                    if cancelled.count > 128 { cancelled.removeFirst() }
                }
                request.resolve()
            case "@impasto/events/start", "@impasto/events/stop":
                guard let control = try? request.decodeJSON(Control.self) else { return false }
                guard control.topic.hasPrefix("\(identifier)/") else { return false }
                if request.operation.hasSuffix("/start") {
                    switch control.topic {
                    case "\(identifier)/position": try startPosition(request, control)
                    case "\(identifier)/service": startService(); request.resolve()
                    default: return false
                    }
                } else {
                    if control.topic == "\(identifier)/position" { position?.stop(); position = nil }
                    if control.topic == "\(identifier)/service" { stopService() }
                    request.resolve()
                }
            default: return false
            }
        } catch { request.fail(code: "platform", message: "Invalid location options") }
        return true
    }

    private func startCurrent(_ request: ImpastoNativeRequest) throws {
        let options = try request.decodeJSON(LocationOptions.self)
        guard let id = options.id, pending[id] == nil else {
            request.fail(code: "position-unavailable", message: "Location request is already active"); return
        }
        if let index = cancelled.firstIndex(of: id) {
            cancelled.remove(at: index)
            request.fail(code: "cancelled", message: "Location request cancelled")
            return
        }
        let tracker = Tracker(options: options)
        let started = Date()
        let freshness = Double(options.maximumAgeMs ?? 0) / 1000
        if let cached = latest ?? tracker.manager.location, cached.horizontalAccuracy >= 0,
           freshness > 0, started.timeIntervalSince(cached.timestamp) <= freshness {
            request.resolveJSON(Position(cached))
            return
        }
        tracker.location = { [weak self] location in
            guard let self, started.timeIntervalSince(location.timestamp) <= freshness,
                  self.pending[id] != nil else { return }
            self.latest = location
            self.take(id)?.resolveJSON(Position(location))
        }
        tracker.error = { [weak self] error in
            if error != .locationUnknown { self?.fail(id, error == .denied ? "permission-denied" : "position-unavailable", "Location could not be determined") }
        }
        let timer = Task { @MainActor [weak self] in
            do { try await Task.sleep(nanoseconds: UInt64(options.timeoutMs ?? 15_000) * 1_000_000) }
            catch { return }
            self?.fail(id, "timeout", "Location request timed out")
        }
        pending[id] = One(request: request, tracker: tracker, timer: timer)
        tracker.manager.startUpdatingLocation()
    }

    private func take(_ id: UInt64) -> ImpastoNativeRequest? {
        guard let operation = pending.removeValue(forKey: id) else { return nil }
        operation.timer.cancel()
        operation.tracker.stop()
        return operation.request
    }
    private func fail(_ id: UInt64, _ code: String, _ message: String) { take(id)?.fail(code: code, message: message) }

    private func startPosition(_ request: ImpastoNativeRequest, _ control: Control) throws {
        let options = try JSONDecoder().decode(LocationOptions.self, from: Data((control.params ?? "{}").utf8))
        let tracker = Tracker(options: options)
        #if os(iOS)
        if options.allowBackground == true {
            let modes = Bundle.main.object(forInfoDictionaryKey: "UIBackgroundModes") as? [String]
            guard background, modes?.contains("location") == true else {
                request.fail(code: "missing-config", message: "Background location requires the location background mode"); return
            }
            tracker.manager.allowsBackgroundLocationUpdates = true
        }
        #endif
        tracker.location = { [weak self] value in
            self?.latest = value
            self?.emit("\(self?.identifier ?? "")/position", Position(value))
        }
        position?.stop()
        position = tracker
        tracker.manager.startUpdatingLocation()
        request.resolve()
    }

    nonisolated private static func serviceEnabled() async -> Bool {
        await Task.detached { CLLocationManager.locationServicesEnabled() }.value
    }
    private func startService() {
        guard service == nil else { return }
        let tracker = Tracker(options: LocationOptions())
        service = tracker
        tracker.authorization = { [weak self] in self?.refreshService() }
        serviceTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            Task { @MainActor in self?.refreshService() }
        }
        refreshService()
    }
    private func refreshService() {
        Task { @MainActor [weak self] in
            let value = await Self.serviceEnabled()
            guard let self, self.service != nil, self.lastService != value else { return }
            self.lastService = value
            self.emit("\(self.identifier)/service", value ? "enabled" : "disabled")
        }
    }
    private func stopService() {
        service?.stop()
        service = nil
        serviceTimer?.invalidate()
        serviceTimer = nil
        lastService = nil
    }
    private func emit(_ topic: String, _ value: some Encodable) {
        if let data = try? JSONEncoder().encode(value) {
            host?.emitPluginEvent(topic: topic, payload: String(decoding: data, as: UTF8.self))
        }
    }

    public func detached() {
        for id in Array(pending.keys) { fail(id, "cancelled", "The location window closed") }
        position?.stop()
        position = nil
        stopService()
    }
    public func uninstalled() { detached() }
}
public protocol ImpastoGeolocatorFeature {}
