import CoreGraphics
import Foundation

public struct StandaloneCursorMotionSegment: Equatable, Sendable {
    public let end: CGPoint
    public let control1: CGPoint
    public let control2: CGPoint

    public init(end: CGPoint, control1: CGPoint, control2: CGPoint) {
        self.end = end
        self.control1 = control1
        self.control2 = control2
    }
}

public struct StandaloneCursorPathSample: Equatable, Sendable {
    public let progress: CGFloat
    public let point: CGPoint
    public let tangent: CGVector
    public let speedUnitsPerProgress: CGFloat

    public init(progress: CGFloat, point: CGPoint, tangent: CGVector, speedUnitsPerProgress: CGFloat) {
        self.progress = progress
        self.point = point
        self.tangent = tangent
        self.speedUnitsPerProgress = speedUnitsPerProgress
    }
}

public struct StandaloneCursorMotionMeasurement: Equatable, Sendable {
    public let length: CGFloat
    public let angleChangeEnergy: CGFloat
    public let maxAngleChange: CGFloat
    public let totalTurn: CGFloat
    public let staysInBounds: Bool

    public init(
        length: CGFloat,
        angleChangeEnergy: CGFloat,
        maxAngleChange: CGFloat,
        totalTurn: CGFloat,
        staysInBounds: Bool
    ) {
        self.length = length
        self.angleChangeEnergy = angleChangeEnergy
        self.maxAngleChange = maxAngleChange
        self.totalTurn = totalTurn
        self.staysInBounds = staysInBounds
    }
}

public struct StandaloneCursorCandidateScoreComponents: Equatable, Sendable {
    public let excessLengthRatio: CGFloat
    public let excessLengthCost: CGFloat
    public let angleEnergyCost: CGFloat
    public let maxAngleCost: CGFloat
    public let totalTurnCost: CGFloat
    public let outOfBoundsCost: CGFloat
    public let totalScore: CGFloat

    public init(
        excessLengthRatio: CGFloat,
        excessLengthCost: CGFloat,
        angleEnergyCost: CGFloat,
        maxAngleCost: CGFloat,
        totalTurnCost: CGFloat,
        outOfBoundsCost: CGFloat,
        totalScore: CGFloat
    ) {
        self.excessLengthRatio = excessLengthRatio
        self.excessLengthCost = excessLengthCost
        self.angleEnergyCost = angleEnergyCost
        self.maxAngleCost = maxAngleCost
        self.totalTurnCost = totalTurnCost
        self.outOfBoundsCost = outOfBoundsCost
        self.totalScore = totalScore
    }
}

public struct StandaloneCursorMotionPath: Equatable, Sendable {
    public let start: CGPoint
    public let end: CGPoint
    public let startControl: CGPoint?
    public let arc: CGPoint?
    public let arcIn: CGPoint?
    public let arcOut: CGPoint?
    public let endControl: CGPoint?
    public let segments: [StandaloneCursorMotionSegment]

    public init(
        start: CGPoint,
        end: CGPoint,
        startControl: CGPoint? = nil,
        arc: CGPoint? = nil,
        arcIn: CGPoint? = nil,
        arcOut: CGPoint? = nil,
        endControl: CGPoint? = nil,
        segments: [StandaloneCursorMotionSegment]
    ) {
        self.start = start
        self.end = end
        self.startControl = startControl
        self.arc = arc
        self.arcIn = arcIn
        self.arcOut = arcOut
        self.endControl = endControl
        self.segments = segments
    }

    public func sample(at progress: CGFloat) -> (point: CGPoint, tangent: CGVector) {
        guard !segments.isEmpty else {
            return (start, CGVector(dx: 1, dy: 0))
        }

        let clamped = progress.clamped(to: 0...1)
        let segmentCount = segments.count
        let segmentIndex: Int
        let localT: CGFloat

        if clamped >= 1 {
            segmentIndex = segmentCount - 1
            localT = 1
        } else {
            let scaled = clamped * CGFloat(segmentCount)
            segmentIndex = min(Int(scaled), segmentCount - 1)
            localT = scaled - CGFloat(segmentIndex)
        }

        let segment = segments[segmentIndex]
        let segmentStart = segmentIndex == 0 ? start : segments[segmentIndex - 1].end
        let point = sampleCubic(
            start: segmentStart,
            control1: segment.control1,
            control2: segment.control2,
            end: segment.end,
            t: localT
        )
        let tangent = sampleCubicTangent(
            start: segmentStart,
            control1: segment.control1,
            control2: segment.control2,
            end: segment.end,
            t: localT
        ).normalized
        return (point, tangent)
    }

    public func samplePoints(count: Int) -> [StandaloneCursorPathSample] {
        let resolvedCount = max(count, 2)
        var samples: [StandaloneCursorPathSample] = []
        samples.reserveCapacity(resolvedCount)
        var previousPoint: CGPoint?

        for index in 0..<resolvedCount {
            let progress = CGFloat(index) / CGFloat(max(resolvedCount - 1, 1))
            let sample = sample(at: progress)
            let speed = previousPoint.map { (sample.point - $0).length } ?? 0
            samples.append(
                StandaloneCursorPathSample(
                    progress: progress,
                    point: sample.point,
                    tangent: sample.tangent,
                    speedUnitsPerProgress: speed
                )
            )
            previousPoint = sample.point
        }

        return samples
    }

    public func measure(
        bounds: CGRect?,
        minStepDistance: CGFloat = 0.01,
        samplesPerSegment: Int = 24
    ) -> StandaloneCursorMotionMeasurement {
        var totalLength: CGFloat = 0
        var angleChangeEnergy: CGFloat = 0
        var maxAngleChange: CGFloat = 0
        var totalTurn: CGFloat = 0
        var staysInBounds = bounds?.contains(start, padding: 20) ?? true
        var previousPoint = start
        var previousAngle: CGFloat?

        let totalSteps = max(segments.count * max(samplesPerSegment, 1), 1)
        for step in 1...totalSteps {
            let progress = CGFloat(step) / CGFloat(totalSteps)
            let point = sample(at: progress).point
            let delta = point - previousPoint
            let stepLength = delta.length

            if let bounds, staysInBounds {
                staysInBounds = bounds.contains(point, padding: 20)
            }

            if stepLength > minStepDistance {
                let angle = atan2(delta.dy, delta.dx)
                totalLength += stepLength

                if let previousAngle {
                    let angleDelta = unwrapAngleDelta(angle - previousAngle)
                    angleChangeEnergy += angleDelta * angleDelta
                    let absoluteDelta = abs(angleDelta)
                    maxAngleChange = max(maxAngleChange, absoluteDelta)
                    totalTurn += absoluteDelta
                }

                previousAngle = angle
                previousPoint = point
            }
        }

        return StandaloneCursorMotionMeasurement(
            length: totalLength,
            angleChangeEnergy: angleChangeEnergy,
            maxAngleChange: maxAngleChange,
            totalTurn: totalTurn,
            staysInBounds: staysInBounds
        )
    }
}

public enum StandaloneCursorCandidateKind: String, Equatable, Sendable {
    case base
    case arched
}

public struct StandaloneCursorCandidate: Identifiable, Equatable, Sendable {
    public let id: String
    public let kind: StandaloneCursorCandidateKind
    public let side: Int
    public let tableAScale: CGFloat?
    public let tableBScale: CGFloat?
    public let score: CGFloat
    public let scoreComponents: StandaloneCursorCandidateScoreComponents
    public let measurement: StandaloneCursorMotionMeasurement
    public let path: StandaloneCursorMotionPath

    public init(
        id: String,
        kind: StandaloneCursorCandidateKind,
        side: Int,
        tableAScale: CGFloat?,
        tableBScale: CGFloat?,
        score: CGFloat,
        scoreComponents: StandaloneCursorCandidateScoreComponents,
        measurement: StandaloneCursorMotionMeasurement,
        path: StandaloneCursorMotionPath
    ) {
        self.id = id
        self.kind = kind
        self.side = side
        self.tableAScale = tableAScale
        self.tableBScale = tableBScale
        self.score = score
        self.scoreComponents = scoreComponents
        self.measurement = measurement
        self.path = path
    }
}

public struct StandaloneCursorSelectionDecision: Equatable, Sendable {
    public let selectedCandidateID: String?
    public let selectionPolicy: String

    public init(selectedCandidateID: String?, selectionPolicy: String) {
        self.selectedCandidateID = selectedCandidateID
        self.selectionPolicy = selectionPolicy
    }
}

public struct StandaloneCursorSpringConfiguration: Equatable, Sendable {
    public let response: CGFloat
    public let dampingFraction: CGFloat
    public let stiffness: CGFloat
    public let drag: CGFloat
    public let dt: CGFloat
    public let closeEnoughProgressThreshold: CGFloat
    public let closeEnoughDistanceThreshold: CGFloat
    public let idleVelocityThreshold: CGFloat

    public init(
        response: CGFloat,
        dampingFraction: CGFloat,
        stiffness: CGFloat,
        drag: CGFloat,
        dt: CGFloat,
        closeEnoughProgressThreshold: CGFloat,
        closeEnoughDistanceThreshold: CGFloat,
        idleVelocityThreshold: CGFloat
    ) {
        self.response = response
        self.dampingFraction = dampingFraction
        self.stiffness = stiffness
        self.drag = drag
        self.dt = dt
        self.closeEnoughProgressThreshold = closeEnoughProgressThreshold
        self.closeEnoughDistanceThreshold = closeEnoughDistanceThreshold
        self.idleVelocityThreshold = idleVelocityThreshold
    }

    public static let official: StandaloneCursorSpringConfiguration = {
        let response: CGFloat = 1.4
        let dampingFraction: CGFloat = 0.9
        let dt: CGFloat = 1.0 / 240.0
        let idleVelocityThreshold: CGFloat = 28_800
        let rawStiffness = response > 0 ? pow((2 * .pi) / response, 2) : .infinity
        let stiffness = min(rawStiffness, idleVelocityThreshold)
        let drag = 2 * dampingFraction * sqrt(stiffness)

        return StandaloneCursorSpringConfiguration(
            response: response,
            dampingFraction: dampingFraction,
            stiffness: stiffness,
            drag: drag,
            dt: dt,
            closeEnoughProgressThreshold: 1,
            closeEnoughDistanceThreshold: 0.01,
            idleVelocityThreshold: idleVelocityThreshold
        )
    }()
}

public struct StandaloneCursorSpringState: Equatable, Sendable {
    public var time: CGFloat
    public var velocity: CGFloat
    public var force: CGFloat

    public init(time: CGFloat = 0, velocity: CGFloat = 0, force: CGFloat = 0) {
        self.time = time
        self.velocity = velocity
        self.force = force
    }
}

public struct StandaloneCursorTimelineSample: Equatable, Identifiable, Sendable {
    public var id: Int { step }

    public let step: Int
    public let time: CGFloat
    public let progress: CGFloat
    public let point: CGPoint
    public let springVelocity: CGFloat
    public let springForce: CGFloat
    public let geometricSpeedUnitsPerSecond: CGFloat

    public init(
        step: Int,
        time: CGFloat,
        progress: CGFloat,
        point: CGPoint,
        springVelocity: CGFloat,
        springForce: CGFloat,
        geometricSpeedUnitsPerSecond: CGFloat
    ) {
        self.step = step
        self.time = time
        self.progress = progress
        self.point = point
        self.springVelocity = springVelocity
        self.springForce = springForce
        self.geometricSpeedUnitsPerSecond = geometricSpeedUnitsPerSecond
    }
}

public struct StandaloneCursorTimeline: Equatable, Sendable {
    public let springConfiguration: StandaloneCursorSpringConfiguration
    public let samples: [StandaloneCursorTimelineSample]
    public let rawProgressFirstGeTargetTime: CGFloat?
    public let firstEndpointLockTime: CGFloat?
    public let closeEnoughFirstTime: CGFloat?
    public let rawProgressFirstGeTargetStep: Int?
    public let firstEndpointLockStep: Int?
    public let closeEnoughFirstStep: Int?
    public let reportEverySteps: Int

    public init(
        springConfiguration: StandaloneCursorSpringConfiguration,
        samples: [StandaloneCursorTimelineSample],
        rawProgressFirstGeTargetTime: CGFloat?,
        firstEndpointLockTime: CGFloat?,
        closeEnoughFirstTime: CGFloat?,
        rawProgressFirstGeTargetStep: Int?,
        firstEndpointLockStep: Int?,
        closeEnoughFirstStep: Int?,
        reportEverySteps: Int
    ) {
        self.springConfiguration = springConfiguration
        self.samples = samples
        self.rawProgressFirstGeTargetTime = rawProgressFirstGeTargetTime
        self.firstEndpointLockTime = firstEndpointLockTime
        self.closeEnoughFirstTime = closeEnoughFirstTime
        self.rawProgressFirstGeTargetStep = rawProgressFirstGeTargetStep
        self.firstEndpointLockStep = firstEndpointLockStep
        self.closeEnoughFirstStep = closeEnoughFirstStep
        self.reportEverySteps = reportEverySteps
    }
}

public enum StandaloneCursorBinaryGuidedModel {
    public static let guideVectorInLocalBasis = CGVector(dx: -0.6946583704589973, dy: 0.7193398003386512)
    public static let tableA: [CGFloat] = [0.55, 0.8, 1.05]
    public static let tableB: [CGFloat] = [0.65, 1.0, 1.35]

    private static let normalizationEpsilon: CGFloat = 0.001
    private static let sideBiasScale: CGFloat = 0.65
    private static let primaryDistanceScale: CGFloat = 0.41960295031576633
    private static let directSpanScale: CGFloat = 0.9
    private static let secondaryDistanceScale: CGFloat = 0.2765523188064277
    private static let arcDistanceScale: CGFloat = 0.5783555327868779
    private static let candidateArcMin: CGFloat = 38
    private static let candidateArcMax: CGFloat = 440
    private static let scoreExcessLengthWeight: CGFloat = 320
    private static let scoreAngleEnergyWeight: CGFloat = 140
    private static let scoreMaxAngleWeight: CGFloat = 180
    private static let scoreTotalTurnWeight: CGFloat = 18
    private static let scoreOutOfBoundsPenalty: CGFloat = 45
    private static let minimumStepDistance: CGFloat = 0.01

    public static func makeCandidates(start: CGPoint, end: CGPoint, bounds: CGRect?) -> [StandaloneCursorCandidate] {
        let delta = end - start
        let distance = max(delta.length, normalizationEpsilon)
        let direction = delta.normalized
        let localNormal = direction.perpendicular
        let guide = direction.scaled(by: guideVectorInLocalBasis.dx)
            + localNormal.scaled(by: guideVectorInLocalBasis.dy)
        let reverseGuide = guide.scaled(by: -1)

        let (startExtentPre, endExtentPre) = binaryPiecewisePrimaryExtents(distance: distance)
        let startExtent = min(startExtentPre, clipPositiveRay(origin: start, direction: guide, bounds: bounds))
        let endExtent = min(endExtentPre, clipPositiveRay(origin: end, direction: reverseGuide, bounds: bounds))

        let startExtentScaled = min(
            max(startExtent * sideBiasScale, 0),
            clipPositiveRay(origin: start, direction: guide, bounds: bounds)
        )
        let endExtentScaled = min(
            max(endExtent * sideBiasScale, 0),
            clipPositiveRay(origin: end, direction: reverseGuide, bounds: bounds)
        )

        let fullStartControl = start + guide.scaled(by: startExtent)
        let fullEndControl = end - guide.scaled(by: endExtent)
        let scaledStartControl = start + guide.scaled(by: startExtentScaled)
        let scaledEndControl = end - guide.scaled(by: endExtentScaled)

        let rawHandleExtent = binaryPiecewiseHandleExtent(distance: distance)
        let rawArcExtent = (distance * arcDistanceScale).clamped(to: candidateArcMin...candidateArcMax)

        let midpoint = CGPoint(x: (start.x + end.x) * 0.5, y: (start.y + end.y) * 0.5)
        var signedNormal = localNormal
        let cross = (guide.dy * direction.dx) - (guide.dx * direction.dy)
        if cross < 0 {
            signedNormal = signedNormal.scaled(by: -1)
        }
        let arcAnchorBias = guide.scaled(by: startExtent * sideBiasScale)
        let forwardUnit = normalizedOrDefault(
            direction.scaled(by: distance) + signedNormal.scaled(by: rawArcExtent),
            minimumLength: rawHandleExtent
        )

        var candidates: [StandaloneCursorCandidate] = []
        candidates.append(
            makeCandidate(
                id: "base-full-guide",
                kind: .base,
                side: 0,
                tableAScale: nil,
                tableBScale: nil,
                path: StandaloneCursorMotionPath(
                    start: start,
                    end: end,
                    startControl: fullStartControl,
                    endControl: fullEndControl,
                    segments: [
                        StandaloneCursorMotionSegment(
                            end: end,
                            control1: fullStartControl,
                            control2: fullEndControl
                        ),
                    ]
                ),
                distance: distance,
                bounds: bounds
            )
        )
        candidates.append(
            makeCandidate(
                id: "base-scaled-guide",
                kind: .base,
                side: 0,
                tableAScale: nil,
                tableBScale: nil,
                path: StandaloneCursorMotionPath(
                    start: start,
                    end: end,
                    startControl: scaledStartControl,
                    endControl: scaledEndControl,
                    segments: [
                        StandaloneCursorMotionSegment(
                            end: end,
                            control1: scaledStartControl,
                            control2: scaledEndControl
                        ),
                    ]
                ),
                distance: distance,
                bounds: bounds
            )
        )

        for outerScale in tableA {
            let anchorOffset = signedNormal.scaled(by: rawHandleExtent * outerScale)
            for innerScale in tableB {
                let tangentSpan = forwardUnit.scaled(by: rawArcExtent * innerScale)

                for side in [1, -1] {
                    let anchor = midpoint + arcAnchorBias + anchorOffset.scaled(by: CGFloat(side))
                    let arcIn = anchor - tangentSpan
                    let arcOut = anchor + tangentSpan
                    let path = StandaloneCursorMotionPath(
                        start: start,
                        end: end,
                        startControl: fullStartControl,
                        arc: anchor,
                        arcIn: arcIn,
                        arcOut: arcOut,
                        endControl: fullEndControl,
                        segments: [
                            StandaloneCursorMotionSegment(end: anchor, control1: fullStartControl, control2: arcIn),
                            StandaloneCursorMotionSegment(end: end, control1: arcOut, control2: fullEndControl),
                        ]
                    )

                    candidates.append(
                        makeCandidate(
                            id: "a\(outerScale.cursorIdentifier)-b\(innerScale.cursorIdentifier)-\(side > 0 ? "positive" : "negative")",
                            kind: .arched,
                            side: side,
                            tableAScale: outerScale,
                            tableBScale: innerScale,
                            path: path,
                            distance: distance,
                            bounds: bounds
                        )
                    )
                }
            }
        }

        return candidates
    }

    public static func chooseCandidate(from candidates: [StandaloneCursorCandidate]) -> StandaloneCursorSelectionDecision {
        guard !candidates.isEmpty else {
            return StandaloneCursorSelectionDecision(selectedCandidateID: nil, selectionPolicy: "empty")
        }

        let inBoundsCandidates = candidates.filter(\.measurement.staysInBounds)
        let pool = inBoundsCandidates.isEmpty ? candidates : inBoundsCandidates
        let selected = pool.min { lhs, rhs in
            if lhs.score == rhs.score {
                return lhs.id < rhs.id
            }
            return lhs.score < rhs.score
        }

        return StandaloneCursorSelectionDecision(
            selectedCandidateID: selected?.id,
            selectionPolicy: inBoundsCandidates.isEmpty ? "lowest_score" : "prefer_in_bounds_then_lowest_score"
        )
    }

    public static func orderedCandidates(from candidates: [StandaloneCursorCandidate]) -> [StandaloneCursorCandidate] {
        candidates.sorted { lhs, rhs in
            if lhs.measurement.staysInBounds != rhs.measurement.staysInBounds {
                return lhs.measurement.staysInBounds && !rhs.measurement.staysInBounds
            }
            if lhs.score == rhs.score {
                return lhs.id < rhs.id
            }
            return lhs.score < rhs.score
        }
    }

    public static func buildTimeline(
        path: StandaloneCursorMotionPath,
        reportEverySteps: Int = 4,
        maxDurationSeconds: CGFloat = 2.0
    ) -> StandaloneCursorTimeline {
        let configuration = StandaloneCursorSpringConfiguration.official
        let stepCount = Int(maxDurationSeconds / configuration.dt)
        var current: CGFloat = 0
        let target: CGFloat = 1
        var state = StandaloneCursorSpringState()
        var samples: [StandaloneCursorTimelineSample] = []
        var previousPoint: CGPoint?
        var rawProgressFirstGeTargetTime: CGFloat?
        var rawProgressFirstGeTargetStep: Int?
        var firstEndpointLockTime: CGFloat?
        var firstEndpointLockStep: Int?
        var closeEnoughFirstTime: CGFloat?
        var closeEnoughFirstStep: Int?

        let startPoint = path.sample(at: current).point
        samples.append(
            StandaloneCursorTimelineSample(
                step: 0,
                time: 0,
                progress: current,
                point: startPoint,
                springVelocity: state.velocity,
                springForce: state.force,
                geometricSpeedUnitsPerSecond: 0
            )
        )
        previousPoint = startPoint

        for step in 1...stepCount {
            let targetTime = CGFloat(step) * configuration.dt
            (current, state) = advanceProgress(
                current: current,
                target: target,
                state: state,
                configuration: configuration,
                to: targetTime
            )

            let point = path.sample(at: current).point
            let geometricSpeed = previousPoint.map { (point - $0).length / configuration.dt } ?? 0
            previousPoint = point

            if rawProgressFirstGeTargetTime == nil, current >= target {
                rawProgressFirstGeTargetTime = state.time
                rawProgressFirstGeTargetStep = step
            }

            if firstEndpointLockTime == nil, current >= target, point == path.end {
                firstEndpointLockTime = state.time
                firstEndpointLockStep = step
            }

            if closeEnoughFirstTime == nil,
               current >= configuration.closeEnoughProgressThreshold,
               abs(target - current) <= configuration.closeEnoughDistanceThreshold {
                closeEnoughFirstTime = state.time
                closeEnoughFirstStep = step
            }

            if (step % reportEverySteps) == 0 || step == stepCount {
                samples.append(
                    StandaloneCursorTimelineSample(
                        step: step,
                        time: state.time,
                        progress: current,
                        point: point,
                        springVelocity: state.velocity,
                        springForce: state.force,
                        geometricSpeedUnitsPerSecond: geometricSpeed
                    )
                )
            }
        }

        return StandaloneCursorTimeline(
            springConfiguration: configuration,
            samples: samples,
            rawProgressFirstGeTargetTime: rawProgressFirstGeTargetTime,
            firstEndpointLockTime: firstEndpointLockTime,
            closeEnoughFirstTime: closeEnoughFirstTime,
            rawProgressFirstGeTargetStep: rawProgressFirstGeTargetStep,
            firstEndpointLockStep: firstEndpointLockStep,
            closeEnoughFirstStep: closeEnoughFirstStep,
            reportEverySteps: reportEverySteps
        )
    }

    public static func advanceProgress(
        current: CGFloat,
        target: CGFloat = 1,
        state: StandaloneCursorSpringState,
        configuration: StandaloneCursorSpringConfiguration = .official
    ) -> (current: CGFloat, state: StandaloneCursorSpringState) {
        let halfDT = configuration.dt * 0.5
        let velocityHalf = state.velocity + (state.force * halfDT)
        let nextCurrent = current + (velocityHalf * configuration.dt)
        let force = (configuration.stiffness * (target - nextCurrent)) + ((-configuration.drag) * velocityHalf)
        let velocity = velocityHalf + (force * halfDT)

        return (
            nextCurrent,
            StandaloneCursorSpringState(
                time: state.time + configuration.dt,
                velocity: velocity,
                force: force
            )
        )
    }

    public static func advanceProgress(
        current: CGFloat,
        target: CGFloat = 1,
        state: StandaloneCursorSpringState,
        configuration: StandaloneCursorSpringConfiguration = .official,
        to targetTime: CGFloat
    ) -> (current: CGFloat, state: StandaloneCursorSpringState) {
        var adjustedState = state
        var adjustedCurrent = current

        if (targetTime - adjustedState.time) > 1 {
            adjustedState.time = targetTime - (1.0 / 60.0)
        }

        while adjustedState.time < targetTime {
            (adjustedCurrent, adjustedState) = advanceProgress(
                current: adjustedCurrent,
                target: target,
                state: adjustedState,
                configuration: configuration
            )
        }

        return (adjustedCurrent, adjustedState)
    }

    private static func makeCandidate(
        id: String,
        kind: StandaloneCursorCandidateKind,
        side: Int,
        tableAScale: CGFloat?,
        tableBScale: CGFloat?,
        path: StandaloneCursorMotionPath,
        distance: CGFloat,
        bounds: CGRect?
    ) -> StandaloneCursorCandidate {
        let measurement = path.measure(bounds: bounds, minStepDistance: minimumStepDistance)
        let scoreComponents = scoreCandidate(distance: distance, measurement: measurement)
        return StandaloneCursorCandidate(
            id: id,
            kind: kind,
            side: side,
            tableAScale: tableAScale,
            tableBScale: tableBScale,
            score: scoreComponents.totalScore,
            scoreComponents: scoreComponents,
            measurement: measurement,
            path: path
        )
    }

    private static func scoreCandidate(
        distance: CGFloat,
        measurement: StandaloneCursorMotionMeasurement
    ) -> StandaloneCursorCandidateScoreComponents {
        let excessLengthRatio = max((measurement.length / max(distance, 1)) - 1, 0)
        let excessLengthCost = excessLengthRatio * scoreExcessLengthWeight
        let angleEnergyCost = measurement.angleChangeEnergy * scoreAngleEnergyWeight
        let maxAngleCost = measurement.maxAngleChange * scoreMaxAngleWeight
        let totalTurnCost = measurement.totalTurn * scoreTotalTurnWeight
        let outOfBoundsCost = measurement.staysInBounds ? 0 : scoreOutOfBoundsPenalty
        let totalScore = excessLengthCost + angleEnergyCost + maxAngleCost + totalTurnCost + outOfBoundsCost
        return StandaloneCursorCandidateScoreComponents(
            excessLengthRatio: excessLengthRatio,
            excessLengthCost: excessLengthCost,
            angleEnergyCost: angleEnergyCost,
            maxAngleCost: maxAngleCost,
            totalTurnCost: totalTurnCost,
            outOfBoundsCost: outOfBoundsCost,
            totalScore: totalScore
        )
    }

    private static func binaryPiecewisePrimaryExtents(distance: CGFloat) -> (startExtent: CGFloat, endExtent: CGFloat) {
        let primary = distance * primaryDistanceScale
        let direct = distance * directSpanScale
        let secondary = distance * 0.15
        let lowCutoff: CGFloat = 48
        let highCutoff: CGFloat = 640

        if primary < lowCutoff {
            return (lowCutoff, lowCutoff)
        }
        if primary < highCutoff {
            return (primary, direct)
        }
        if secondary < highCutoff {
            return (highCutoff, lowCutoff)
        }
        return (highCutoff, highCutoff)
    }

    private static func binaryPiecewiseHandleExtent(distance: CGFloat) -> CGFloat {
        let raw = distance * secondaryDistanceScale
        if raw < 50 {
            return 50
        }
        if raw < 640 {
            return raw
        }
        return 520
    }

    private static func clipPositiveRay(origin: CGPoint, direction: CGVector, bounds: CGRect?) -> CGFloat {
        guard let bounds else {
            return .infinity
        }

        var limit = CGFloat.infinity
        if direction.dx > 0 {
            limit = min(limit, (bounds.maxX - origin.x) / direction.dx)
        } else if direction.dx < 0 {
            limit = min(limit, (bounds.minX - origin.x) / direction.dx)
        }

        if direction.dy > 0 {
            limit = min(limit, (bounds.maxY - origin.y) / direction.dy)
        } else if direction.dy < 0 {
            limit = min(limit, (bounds.minY - origin.y) / direction.dy)
        }

        return max(limit, 0)
    }

    private static func normalizedOrDefault(_ vector: CGVector, minimumLength: CGFloat) -> CGVector {
        let length = vector.length
        if length < minimumLength || length < normalizationEpsilon {
            return CGVector(dx: 1, dy: 0)
        }
        return vector.scaled(by: 1 / length)
    }
}

private func sampleCubic(
    start: CGPoint,
    control1: CGPoint,
    control2: CGPoint,
    end: CGPoint,
    t: CGFloat
) -> CGPoint {
    let omt = 1 - t
    let omt2 = omt * omt
    let t2 = t * t
    return CGPoint(
        x: (omt2 * omt * start.x)
            + (3 * omt2 * t * control1.x)
            + (3 * omt * t2 * control2.x)
            + (t2 * t * end.x),
        y: (omt2 * omt * start.y)
            + (3 * omt2 * t * control1.y)
            + (3 * omt * t2 * control2.y)
            + (t2 * t * end.y)
    )
}

private func sampleCubicTangent(
    start: CGPoint,
    control1: CGPoint,
    control2: CGPoint,
    end: CGPoint,
    t: CGFloat
) -> CGVector {
    let omt = 1 - t
    return CGVector(
        dx: (3 * omt * omt * (control1.x - start.x))
            + (6 * omt * t * (control2.x - control1.x))
            + (3 * t * t * (end.x - control2.x)),
        dy: (3 * omt * omt * (control1.y - start.y))
            + (6 * omt * t * (control2.y - control1.y))
            + (3 * t * t * (end.y - control2.y))
    )
}

private func unwrapAngleDelta(_ value: CGFloat) -> CGFloat {
    var wrapped = value
    while wrapped > .pi {
        wrapped -= (.pi * 2)
    }
    while wrapped < -.pi {
        wrapped += (.pi * 2)
    }
    return wrapped
}

private extension CGRect {
    func contains(_ point: CGPoint, padding: CGFloat) -> Bool {
        point.x >= minX - padding
            && point.x <= maxX + padding
            && point.y >= minY - padding
            && point.y <= maxY + padding
    }
}

private extension CGPoint {
    static func + (lhs: CGPoint, rhs: CGVector) -> CGPoint {
        CGPoint(x: lhs.x + rhs.dx, y: lhs.y + rhs.dy)
    }

    static func - (lhs: CGPoint, rhs: CGVector) -> CGPoint {
        CGPoint(x: lhs.x - rhs.dx, y: lhs.y - rhs.dy)
    }

    static func - (lhs: CGPoint, rhs: CGPoint) -> CGVector {
        CGVector(dx: lhs.x - rhs.x, dy: lhs.y - rhs.y)
    }
}

private extension CGVector {
    static func + (lhs: CGVector, rhs: CGVector) -> CGVector {
        CGVector(dx: lhs.dx + rhs.dx, dy: lhs.dy + rhs.dy)
    }

    static func - (lhs: CGVector, rhs: CGVector) -> CGVector {
        CGVector(dx: lhs.dx - rhs.dx, dy: lhs.dy - rhs.dy)
    }

    var length: CGFloat {
        sqrt((dx * dx) + (dy * dy))
    }

    var normalized: CGVector {
        let resolvedLength = max(length, 0.000_001)
        return CGVector(dx: dx / resolvedLength, dy: dy / resolvedLength)
    }

    var perpendicular: CGVector {
        CGVector(dx: -dy, dy: dx)
    }

    func scaled(by factor: CGFloat) -> CGVector {
        CGVector(dx: dx * factor, dy: dy * factor)
    }
}

private extension CGFloat {
    func clamped(to range: ClosedRange<CGFloat>) -> CGFloat {
        Swift.min(Swift.max(self, range.lowerBound), range.upperBound)
    }

    var cursorIdentifier: String {
        String(format: "%.2f", Double(self)).replacingOccurrences(of: ".", with: "")
    }
}
