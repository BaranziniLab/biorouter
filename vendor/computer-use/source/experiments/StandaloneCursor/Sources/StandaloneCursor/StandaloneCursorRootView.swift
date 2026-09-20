import AppKit
import QuartzCore
import SwiftUI
import StandaloneCursorSupport

struct StandaloneCursorRootView: View {
    @StateObject private var model = StandaloneCursorViewModel()

    var body: some View {
        HStack(spacing: 0) {
            GeometryReader { proxy in
                let bounds = StandaloneCursorViewModel.canvasBounds(for: proxy.size)

                ZStack(alignment: .topLeading) {
                    canvasBackground

                    RoundedRectangle(cornerRadius: 34, style: .continuous)
                        .fill(
                            LinearGradient(
                                colors: [
                                    Color(red: 0.07, green: 0.11, blue: 0.16),
                                    Color(red: 0.04, green: 0.07, blue: 0.11),
                                ],
                                startPoint: .topLeading,
                                endPoint: .bottomTrailing
                            )
                        )
                        .overlay(
                            RoundedRectangle(cornerRadius: 34, style: .continuous)
                                .stroke(Color.white.opacity(0.08), lineWidth: 1)
                        )
                        .padding(20)

                    BoundsBackdrop(bounds: bounds)

                    if model.showAllCandidates {
                        ForEach(model.orderedCandidates) { candidate in
                            MotionCurveShape(motionPath: candidate.path)
                                .stroke(
                                    candidate.id == model.selectedCandidate?.id
                                        ? Color(red: 1.0, green: 0.72, blue: 0.28).opacity(0.42)
                                        : candidate.measurement.staysInBounds
                                            ? Color(red: 0.53, green: 0.78, blue: 0.90).opacity(0.20)
                                            : Color.white.opacity(0.08),
                                    lineWidth: candidate.id == model.selectedCandidate?.id ? 3.5 : 1.1
                                )
                        }
                    }

                    if let selected = model.selectedCandidate {
                        MotionCurveShape(motionPath: selected.path)
                            .stroke(
                                LinearGradient(
                                    colors: [
                                        Color(red: 0.98, green: 0.81, blue: 0.33),
                                        Color(red: 0.95, green: 0.50, blue: 0.20),
                                    ],
                                    startPoint: .leading,
                                    endPoint: .trailing
                                ),
                                style: StrokeStyle(lineWidth: 4.5, lineCap: .round, lineJoin: .round)
                            )

                        MotionTrailShape(motionPath: selected.path, progress: model.displayProgress)
                            .stroke(
                                LinearGradient(
                                    colors: [
                                        Color(red: 0.73, green: 0.94, blue: 0.98),
                                        Color(red: 0.36, green: 0.73, blue: 0.89),
                                    ],
                                    startPoint: .leading,
                                    endPoint: .trailing
                                ),
                                style: StrokeStyle(lineWidth: 7, lineCap: .round, lineJoin: .round)
                            )
                            .shadow(color: Color(red: 0.36, green: 0.73, blue: 0.89).opacity(0.26), radius: 16)

                        if model.showControlPolygon {
                            ControlPolygonLayer(candidate: selected)
                        }
                    }

                    HandleView(label: "START", tint: Color(red: 0.35, green: 0.86, blue: 0.80))
                        .offset(x: model.start.x - 18, y: model.start.y - 18)
                        .gesture(
                            DragGesture(minimumDistance: 0)
                                .onChanged { value in
                                    model.setStart(to: value.location, livePreview: true)
                                }
                                .onEnded { value in
                                    model.setStart(to: value.location, livePreview: false)
                                }
                        )

                    HandleView(label: "END", tint: Color(red: 1.0, green: 0.65, blue: 0.26))
                        .offset(x: model.end.x - 18, y: model.end.y - 18)
                        .gesture(
                            DragGesture(minimumDistance: 0)
                                .onChanged { value in
                                    model.setEnd(to: value.location, livePreview: true)
                                }
                                .onEnded { value in
                                    model.setEnd(to: value.location, livePreview: false)
                                }
                        )

                    CursorGlyphView(heading: model.headingAngle)
                        .offset(x: model.currentPoint.x, y: model.currentPoint.y)

                    statusCard
                        .padding(28)
                }
                .onAppear {
                    model.updateCanvasBounds(bounds)
                }
                .onChange(of: proxy.size) { _, newSize in
                    model.updateCanvasBounds(StandaloneCursorViewModel.canvasBounds(for: newSize))
                }
            }

            sidebar
                .frame(width: 336)
                .background(
                    LinearGradient(
                        colors: [
                            Color(red: 0.10, green: 0.08, blue: 0.06),
                            Color(red: 0.06, green: 0.06, blue: 0.08),
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
        }
    }

    private var canvasBackground: some View {
        LinearGradient(
            colors: [
                Color(red: 0.12, green: 0.16, blue: 0.21),
                Color(red: 0.08, green: 0.11, blue: 0.16),
                Color(red: 0.04, green: 0.05, blue: 0.07),
            ],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .overlay(alignment: .topLeading) {
            Circle()
                .fill(Color(red: 1.0, green: 0.58, blue: 0.18).opacity(0.13))
                .frame(width: 360, height: 360)
                .blur(radius: 60)
                .offset(x: -110, y: -80)
        }
        .overlay(alignment: .bottomTrailing) {
            Circle()
                .fill(Color(red: 0.28, green: 0.72, blue: 0.92).opacity(0.16))
                .frame(width: 320, height: 320)
                .blur(radius: 52)
                .offset(x: 80, y: 100)
        }
    }

    private var statusCard: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("BINARY-GUIDED STANDALONE CURSOR")
                .font(.system(size: 12, weight: .black, design: .rounded))
                .tracking(0.9)
                .foregroundStyle(Color.white.opacity(0.92))

            Text(model.selectionModeLabel)
                .font(.system(size: 11, weight: .semibold, design: .rounded))
                .tracking(0.4)
                .foregroundStyle(Color(red: 0.98, green: 0.79, blue: 0.35))

            Text(model.selectedCandidateSummary)
                .font(.system(size: 13, weight: .medium, design: .rounded))
                .foregroundStyle(Color.white.opacity(0.78))

            Text(model.timingSummary)
                .font(.system(size: 11, weight: .medium, design: .rounded))
                .foregroundStyle(Color.white.opacity(0.56))

            Text("Ground truth here is the reconstructed path pool, measurement, score formula, and the raw 1.4 / 0.9 spring chain from the Python script. Duration mapping is still intentionally not claimed as recovered.")
                .font(.system(size: 11, weight: .regular, design: .rounded))
                .foregroundStyle(Color.white.opacity(0.48))
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 360, alignment: .leading)
        }
        .padding(18)
        .background(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(Color.black.opacity(0.26))
                .overlay(
                    RoundedRectangle(cornerRadius: 22, style: .continuous)
                        .stroke(Color.white.opacity(0.08), lineWidth: 1)
                )
        )
    }

    private var sidebar: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("StandaloneCursor")
                        .font(.system(size: 24, weight: .black, design: .rounded))
                        .foregroundStyle(Color.white.opacity(0.96))
                    Text("A new standalone viewer built from `scripts/cursor-motion-re/official_cursor_motion.py`, with the visual surface stripped back to the binary-lifted path pool, score model, and spring timeline.")
                        .font(.system(size: 12, weight: .medium, design: .rounded))
                        .foregroundStyle(Color.white.opacity(0.62))
                        .fixedSize(horizontal: false, vertical: true)
                }

                VStack(alignment: .leading, spacing: 10) {
                    HStack(spacing: 10) {
                        ActionButton(title: "Replay", accent: .warm) {
                            model.replay()
                        }
                        ActionButton(title: "Auto", accent: .cool) {
                            model.useAutomaticSelection()
                        }
                    }

                    Toggle("Show all candidates", isOn: $model.showAllCandidates)
                    Toggle("Show control polygon", isOn: $model.showControlPolygon)
                }
                .toggleStyle(StandaloneCursorToggleStyle())
                .padding(16)
                .background(cardBackground)

                VStack(alignment: .leading, spacing: 8) {
                    MetricRow(label: "Selection policy", value: model.selectionPolicy)
                    MetricRow(label: "Selected", value: model.selectedCandidate?.id ?? "none")
                    MetricRow(label: "In bounds", value: model.selectedCandidate?.measurement.staysInBounds == true ? "true" : "false")
                    MetricRow(label: "Raw progress", value: model.formatted(model.rawProgress))
                    MetricRow(label: "Display progress", value: model.formatted(model.displayProgress))
                    MetricRow(label: "Elapsed", value: "\(model.formatted(model.playbackTime)) s")
                }
                .padding(16)
                .background(cardBackground)

                VStack(alignment: .leading, spacing: 8) {
                    Text("Timing")
                        .font(.system(size: 14, weight: .bold, design: .rounded))
                        .foregroundStyle(Color.white.opacity(0.88))
                    MetricRow(label: "Endpoint lock", value: model.timelineMetricString(model.timeline?.firstEndpointLockTime))
                    MetricRow(label: "Close enough", value: model.timelineMetricString(model.timeline?.closeEnoughFirstTime))
                    MetricRow(label: "Reported hz", value: model.timeline.map { "\((Int(round(1 / ($0.springConfiguration.dt * CGFloat($0.reportEverySteps))))))" } ?? "-")
                    Text("The app replays raw spring time directly. There is no speculative distance-based duration layer here.")
                        .font(.system(size: 11, weight: .medium, design: .rounded))
                        .foregroundStyle(Color.white.opacity(0.52))
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(16)
                .background(cardBackground)

                VStack(alignment: .leading, spacing: 10) {
                    Text("Candidates")
                        .font(.system(size: 14, weight: .bold, design: .rounded))
                        .foregroundStyle(Color.white.opacity(0.88))

                    ForEach(model.orderedCandidates) { candidate in
                        Button {
                            model.selectCandidate(candidate.id)
                        } label: {
                            CandidateRow(candidate: candidate, isSelected: candidate.id == model.selectedCandidate?.id)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(16)
                .background(cardBackground)
            }
            .padding(20)
        }
    }

    private var cardBackground: some View {
        RoundedRectangle(cornerRadius: 24, style: .continuous)
            .fill(Color.white.opacity(0.05))
            .overlay(
                RoundedRectangle(cornerRadius: 24, style: .continuous)
                    .stroke(Color.white.opacity(0.08), lineWidth: 1)
            )
    }
}

@MainActor
final class StandaloneCursorViewModel: ObservableObject {
    @Published private(set) var canvasBounds = CGRect(x: 72, y: 72, width: 840, height: 580)
    @Published private(set) var start = CGPoint(x: 180, y: 480)
    @Published private(set) var end = CGPoint(x: 760, y: 220)
    @Published private(set) var orderedCandidates: [StandaloneCursorCandidate] = []
    @Published private(set) var selectedCandidate: StandaloneCursorCandidate?
    @Published private(set) var selectionPolicy = "prefer_in_bounds_then_lowest_score"
    @Published private(set) var timeline: StandaloneCursorTimeline?
    @Published private(set) var currentPoint = CGPoint(x: 180, y: 480)
    @Published private(set) var headingAngle: CGFloat = 0
    @Published private(set) var rawProgress: CGFloat = 0
    @Published private(set) var displayProgress: CGFloat = 0
    @Published private(set) var playbackTime: CGFloat = 0
    @Published var showAllCandidates = true
    @Published var showControlPolygon = true

    private var selectionMode: SelectionMode = .automatic
    private var springState = StandaloneCursorSpringState()
    private var displayLink: CVDisplayLink?
    private var lastTimestamp: CFTimeInterval?
    private var isAnimating = false

    enum SelectionMode {
        case automatic
        case manual(String)
    }

    init() {
        rebuildSelection(replay: false)
        ensureDisplayLink()
    }

    static func canvasBounds(for size: CGSize) -> CGRect {
        let insetX: CGFloat = 72
        let insetY: CGFloat = 78
        let width = max(size.width - (insetX * 2), 240)
        let height = max(size.height - (insetY * 2), 240)
        return CGRect(x: insetX, y: insetY, width: width, height: height)
    }

    var selectionModeLabel: String {
        switch selectionMode {
        case .automatic:
            return "AUTO SELECT • \(selectionPolicy.uppercased())"
        case let .manual(id):
            return "MANUAL LOCK • \(id.uppercased())"
        }
    }

    var selectedCandidateSummary: String {
        guard let selectedCandidate else {
            return "20 candidates are generated from the recovered 2 base + 18 arched pool."
        }

        return String(
            format: "%@ • score %.2f • length %.1f • total turn %.2f",
            selectedCandidate.id,
            Double(selectedCandidate.score),
            Double(selectedCandidate.measurement.length),
            Double(selectedCandidate.measurement.totalTurn)
        )
    }

    var timingSummary: String {
        guard let timeline else {
            return "raw spring timeline unavailable"
        }

        return "endpoint lock \(timelineMetricString(timeline.firstEndpointLockTime)) • close enough \(timelineMetricString(timeline.closeEnoughFirstTime))"
    }

    func updateCanvasBounds(_ bounds: CGRect) {
        guard canvasBounds != bounds else {
            return
        }

        canvasBounds = bounds
        start = clamp(start)
        end = clamp(end)
        rebuildSelection(replay: false)
    }

    func setStart(to location: CGPoint, livePreview: Bool) {
        start = clamp(location)
        rebuildSelection(replay: !livePreview)
    }

    func setEnd(to location: CGPoint, livePreview: Bool) {
        end = clamp(location)
        rebuildSelection(replay: !livePreview)
    }

    func selectCandidate(_ id: String) {
        selectionMode = .manual(id)
        rebuildSelection(replay: true)
    }

    func useAutomaticSelection() {
        selectionMode = .automatic
        rebuildSelection(replay: true)
    }

    func replay() {
        startPlayback()
    }

    func formatted(_ value: CGFloat) -> String {
        String(format: "%.3f", Double(value))
    }

    func timelineMetricString(_ value: CGFloat?) -> String {
        guard let value else {
            return "-"
        }
        return "\(formatted(value)) s"
    }

    private func rebuildSelection(replay: Bool) {
        let candidates = StandaloneCursorBinaryGuidedModel.makeCandidates(start: start, end: end, bounds: canvasBounds)
        let decision = StandaloneCursorBinaryGuidedModel.chooseCandidate(from: candidates)
        selectionPolicy = decision.selectionPolicy
        orderedCandidates = StandaloneCursorBinaryGuidedModel.orderedCandidates(from: candidates)

        switch selectionMode {
        case .automatic:
            selectedCandidate = orderedCandidates.first(where: { $0.id == decision.selectedCandidateID }) ?? orderedCandidates.first
        case let .manual(id):
            selectedCandidate = orderedCandidates.first(where: { $0.id == id }) ?? orderedCandidates.first
            if let selectedCandidate {
                selectionMode = .manual(selectedCandidate.id)
            } else {
                selectionMode = .automatic
            }
        }

        timeline = selectedCandidate.map {
            StandaloneCursorBinaryGuidedModel.buildTimeline(path: $0.path)
        }

        if replay {
            startPlayback()
        } else {
            snapToStart()
        }
    }

    private func snapToStart() {
        lastTimestamp = nil
        isAnimating = false
        playbackTime = 0
        rawProgress = 0
        displayProgress = 0
        springState = StandaloneCursorSpringState()

        if let sample = selectedCandidate?.path.sample(at: 0) {
            currentPoint = sample.point
            headingAngle = atan2(sample.tangent.dy, sample.tangent.dx)
        } else {
            currentPoint = start
            headingAngle = 0
        }
    }

    private func startPlayback() {
        guard selectedCandidate != nil else {
            return
        }

        snapToStart()
        isAnimating = true
    }

    private func ensureDisplayLink() {
        guard displayLink == nil else {
            return
        }

        var link: CVDisplayLink?
        CVDisplayLinkCreateWithActiveCGDisplays(&link)
        guard let link else {
            return
        }

        let callback: CVDisplayLinkOutputCallback = { _, _, _, _, _, userInfo in
            guard let userInfo else {
                return kCVReturnSuccess
            }

            let viewModel = Unmanaged<StandaloneCursorViewModel>.fromOpaque(userInfo).takeUnretainedValue()
            Task { @MainActor in
                viewModel.tick()
            }
            return kCVReturnSuccess
        }

        CVDisplayLinkSetOutputCallback(link, callback, Unmanaged.passUnretained(self).toOpaque())
        displayLink = link
        CVDisplayLinkStart(link)
    }

    private func tick() {
        guard isAnimating, let selectedCandidate else {
            lastTimestamp = nil
            return
        }

        let now = CACurrentMediaTime()
        let dt = CGFloat(lastTimestamp.map { now - $0 } ?? (1.0 / 60.0))
        lastTimestamp = now

        playbackTime += max(1.0 / 240.0, min(dt, 1.0 / 24.0))
        (rawProgress, springState) = StandaloneCursorBinaryGuidedModel.advanceProgress(
            current: rawProgress,
            state: springState,
            to: playbackTime
        )

        let sample = selectedCandidate.path.sample(at: rawProgress)
        currentPoint = sample.point
        headingAngle = atan2(sample.tangent.dy, sample.tangent.dx)
        displayProgress = min(max(rawProgress, 0), 1)

        if let closeEnoughTime = timeline?.closeEnoughFirstTime, playbackTime >= closeEnoughTime {
            isAnimating = false
        }
    }

    private func clamp(_ point: CGPoint) -> CGPoint {
        CGPoint(
            x: min(max(point.x, canvasBounds.minX), canvasBounds.maxX),
            y: min(max(point.y, canvasBounds.minY), canvasBounds.maxY)
        )
    }
}

private struct BoundsBackdrop: View {
    let bounds: CGRect

    var body: some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 30, style: .continuous)
                .stroke(Color.white.opacity(0.08), style: StrokeStyle(lineWidth: 1, dash: [12, 10]))
                .frame(width: bounds.width, height: bounds.height)
                .offset(x: bounds.minX, y: bounds.minY)

            RoundedRectangle(cornerRadius: 30, style: .continuous)
                .fill(Color.white.opacity(0.02))
                .frame(width: bounds.width, height: bounds.height)
                .offset(x: bounds.minX, y: bounds.minY)
        }
    }
}

private struct MotionCurveShape: Shape {
    let motionPath: StandaloneCursorMotionPath

    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: motionPath.start)

        for segment in motionPath.segments {
            path.addCurve(to: segment.end, control1: segment.control1, control2: segment.control2)
        }

        return path
    }
}

private struct MotionTrailShape: Shape {
    let motionPath: StandaloneCursorMotionPath
    let progress: CGFloat

    func path(in rect: CGRect) -> Path {
        let sampleCount = max(Int(progress * 96), 2)
        var path = Path()
        path.move(to: motionPath.start)

        for index in 1...sampleCount {
            let fraction = CGFloat(index) / CGFloat(sampleCount)
            let point = motionPath.sample(at: progress * fraction).point
            path.addLine(to: point)
        }

        return path
    }
}

private struct ControlPolygonLayer: View {
    let candidate: StandaloneCursorCandidate

    var body: some View {
        ZStack(alignment: .topLeading) {
            if let startControl = candidate.path.startControl {
                Path { path in
                    path.move(to: candidate.path.start)
                    path.addLine(to: startControl)
                }
                .stroke(Color.white.opacity(0.18), style: StrokeStyle(lineWidth: 1, dash: [5, 5]))

                ControlNode(point: startControl, tint: Color.white.opacity(0.7))
            }

            if let arc = candidate.path.arc {
                if let arcIn = candidate.path.arcIn {
                    Path { path in
                        path.move(to: arc)
                        path.addLine(to: arcIn)
                    }
                    .stroke(Color.white.opacity(0.16), style: StrokeStyle(lineWidth: 1, dash: [4, 6]))

                    ControlNode(point: arcIn, tint: Color(red: 0.36, green: 0.73, blue: 0.89))
                }

                if let arcOut = candidate.path.arcOut {
                    Path { path in
                        path.move(to: arc)
                        path.addLine(to: arcOut)
                    }
                    .stroke(Color.white.opacity(0.16), style: StrokeStyle(lineWidth: 1, dash: [4, 6]))

                    ControlNode(point: arcOut, tint: Color(red: 0.36, green: 0.73, blue: 0.89))
                }

                ArcAnchorNode(point: arc)
            }

            if let endControl = candidate.path.endControl {
                Path { path in
                    path.move(to: endControl)
                    path.addLine(to: candidate.path.end)
                }
                .stroke(Color.white.opacity(0.18), style: StrokeStyle(lineWidth: 1, dash: [5, 5]))

                ControlNode(point: endControl, tint: Color.white.opacity(0.7))
            }
        }
    }
}

private struct ControlNode: View {
    let point: CGPoint
    let tint: Color

    var body: some View {
        Circle()
            .fill(tint)
            .frame(width: 9, height: 9)
            .overlay(Circle().stroke(Color.black.opacity(0.45), lineWidth: 1))
            .offset(x: point.x - 4.5, y: point.y - 4.5)
    }
}

private struct ArcAnchorNode: View {
    let point: CGPoint

    var body: some View {
        DiamondShape()
            .fill(Color(red: 0.98, green: 0.79, blue: 0.35))
            .frame(width: 12, height: 12)
            .overlay(DiamondShape().stroke(Color.black.opacity(0.45), lineWidth: 1))
            .offset(x: point.x - 6, y: point.y - 6)
    }
}

private struct HandleView: View {
    let label: String
    let tint: Color

    var body: some View {
        ZStack(alignment: .topLeading) {
            Circle()
                .fill(tint.opacity(0.18))
                .frame(width: 36, height: 36)
                .overlay(
                    Circle()
                        .stroke(tint, lineWidth: 2)
                )
                .shadow(color: tint.opacity(0.28), radius: 12)

            Text(label)
                .font(.system(size: 10, weight: .black, design: .rounded))
                .tracking(0.8)
                .foregroundStyle(tint)
                .padding(.horizontal, 8)
                .padding(.vertical, 4)
                .background(
                    Capsule(style: .continuous)
                        .fill(Color.black.opacity(0.32))
                        .overlay(Capsule(style: .continuous).stroke(Color.white.opacity(0.08), lineWidth: 1))
                )
                .offset(x: 22, y: -10)
        }
        .frame(width: 110, height: 44, alignment: .topLeading)
    }
}

private struct CursorGlyphView: View {
    let heading: CGFloat

    var body: some View {
        ZStack(alignment: .topLeading) {
            CursorGlyphShape()
                .fill(
                    LinearGradient(
                        colors: [
                            Color.white,
                            Color(red: 0.89, green: 0.90, blue: 0.95),
                        ],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .overlay(
                    CursorGlyphShape()
                        .stroke(Color.black.opacity(0.55), lineWidth: 1.1)
                )
                .shadow(color: Color.black.opacity(0.28), radius: 18, x: 0, y: 8)
        }
        .frame(width: 30, height: 40, alignment: .topLeading)
        .rotationEffect(.radians(heading), anchor: .topLeading)
    }
}

private struct CursorGlyphShape: Shape {
    func path(in rect: CGRect) -> Path {
        let width = rect.width
        let height = rect.height

        var path = Path()
        path.move(to: CGPoint(x: 0, y: 0))
        path.addLine(to: CGPoint(x: width * 0.58, y: height * 0.60))
        path.addLine(to: CGPoint(x: width * 0.38, y: height * 0.62))
        path.addLine(to: CGPoint(x: width * 0.50, y: height))
        path.addLine(to: CGPoint(x: width * 0.30, y: height))
        path.addLine(to: CGPoint(x: width * 0.18, y: height * 0.66))
        path.addLine(to: CGPoint(x: 0, y: height * 0.80))
        path.closeSubpath()
        return path
    }
}

private struct DiamondShape: Shape {
    func path(in rect: CGRect) -> Path {
        var path = Path()
        path.move(to: CGPoint(x: rect.midX, y: rect.minY))
        path.addLine(to: CGPoint(x: rect.maxX, y: rect.midY))
        path.addLine(to: CGPoint(x: rect.midX, y: rect.maxY))
        path.addLine(to: CGPoint(x: rect.minX, y: rect.midY))
        path.closeSubpath()
        return path
    }
}

private struct MetricRow: View {
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(label.uppercased())
                .font(.system(size: 10, weight: .black, design: .rounded))
                .tracking(0.8)
                .foregroundStyle(Color.white.opacity(0.45))

            Spacer(minLength: 12)

            Text(value)
                .font(.system(size: 12, weight: .semibold, design: .rounded))
                .foregroundStyle(Color.white.opacity(0.82))
        }
    }
}

private struct CandidateRow: View {
    let candidate: StandaloneCursorCandidate
    let isSelected: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(candidate.id.uppercased())
                    .font(.system(size: 11, weight: .black, design: .rounded))
                    .tracking(0.6)
                Spacer()
                Text(candidate.measurement.staysInBounds ? "IN" : "OUT")
                    .font(.system(size: 10, weight: .black, design: .rounded))
                    .tracking(0.8)
            }

            Text(String(format: "score %.2f • len %.1f • turn %.2f", Double(candidate.score), Double(candidate.measurement.length), Double(candidate.measurement.totalTurn)))
                .font(.system(size: 11, weight: .medium, design: .rounded))
                .foregroundStyle(Color.white.opacity(0.72))
        }
        .foregroundStyle(isSelected ? Color.black.opacity(0.82) : Color.white.opacity(0.92))
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(
                    isSelected
                        ? Color(red: 0.98, green: 0.79, blue: 0.35)
                        : Color.white.opacity(0.05)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .stroke(Color.white.opacity(isSelected ? 0 : 0.08), lineWidth: 1)
                )
        )
    }
}

private struct ActionButton: View {
    enum Accent {
        case warm
        case cool
    }

    let title: String
    let accent: Accent
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            Text(title.uppercased())
                .font(.system(size: 11, weight: .black, design: .rounded))
                .tracking(0.8)
                .frame(maxWidth: .infinity)
                .padding(.vertical, 11)
        }
        .buttonStyle(.plain)
        .background(background)
        .foregroundStyle(Color.black.opacity(0.82))
        .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }

    private var background: some View {
        let colors: [Color] = switch accent {
        case .warm:
            [Color(red: 0.98, green: 0.79, blue: 0.35), Color(red: 0.96, green: 0.57, blue: 0.19)]
        case .cool:
            [Color(red: 0.66, green: 0.92, blue: 0.93), Color(red: 0.35, green: 0.72, blue: 0.89)]
        }

        return LinearGradient(colors: colors, startPoint: .leading, endPoint: .trailing)
    }
}

private struct StandaloneCursorToggleStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        Button {
            configuration.isOn.toggle()
        } label: {
            HStack {
                configuration.label
                    .font(.system(size: 12, weight: .semibold, design: .rounded))
                Spacer()
                RoundedRectangle(cornerRadius: 999, style: .continuous)
                    .fill(configuration.isOn ? Color(red: 0.36, green: 0.73, blue: 0.89) : Color.white.opacity(0.12))
                    .frame(width: 42, height: 24)
                    .overlay(alignment: configuration.isOn ? .trailing : .leading) {
                        Circle()
                            .fill(Color.white)
                            .frame(width: 18, height: 18)
                            .padding(3)
                    }
            }
            .foregroundStyle(Color.white.opacity(0.82))
        }
        .buttonStyle(.plain)
    }
}
