import CoreGraphics
import XCTest
@testable import StandaloneCursorSupport

final class StandaloneCursorSupportTests: XCTestCase {
    func testRecoveredCandidatePoolShape() {
        let candidates = StandaloneCursorBinaryGuidedModel.makeCandidates(
            start: CGPoint(x: 120, y: 180),
            end: CGPoint(x: 760, y: 320),
            bounds: CGRect(x: 0, y: 0, width: 960, height: 640)
        )

        XCTAssertEqual(candidates.count, 20)
        XCTAssertEqual(candidates.filter { $0.kind == .base }.count, 2)
        XCTAssertEqual(candidates.filter { $0.kind == .arched }.count, 18)
    }

    func testSelectionPrefersInBoundsCandidatesWhenAvailable() {
        let candidates = StandaloneCursorBinaryGuidedModel.makeCandidates(
            start: CGPoint(x: 120, y: 120),
            end: CGPoint(x: 760, y: 340),
            bounds: CGRect(x: 0, y: 0, width: 840, height: 420)
        )
        let decision = StandaloneCursorBinaryGuidedModel.chooseCandidate(from: candidates)
        let selected = candidates.first { $0.id == decision.selectedCandidateID }

        XCTAssertNotNil(selected)

        let inBounds = candidates.filter(\.measurement.staysInBounds)
        if inBounds.isEmpty == false {
            XCTAssertEqual(decision.selectionPolicy, "prefer_in_bounds_then_lowest_score")
            XCTAssertEqual(selected?.measurement.staysInBounds, true)
        }
    }

    func testRecoveredSpringTimelineLocksEndpointBeforeCloseEnough() throws {
        let candidates = StandaloneCursorBinaryGuidedModel.makeCandidates(
            start: CGPoint(x: 100, y: 120),
            end: CGPoint(x: 720, y: 380),
            bounds: CGRect(x: 0, y: 0, width: 1280, height: 800)
        )
        let selectedID = StandaloneCursorBinaryGuidedModel.chooseCandidate(from: candidates).selectedCandidateID
        let candidate = candidates.first { $0.id == selectedID }

        XCTAssertNotNil(candidate)

        let timeline = StandaloneCursorBinaryGuidedModel.buildTimeline(path: try XCTUnwrap(candidate?.path))
        XCTAssertNotNil(timeline.firstEndpointLockTime)
        XCTAssertNotNil(timeline.closeEnoughFirstTime)
        XCTAssertLessThanOrEqual(try XCTUnwrap(timeline.firstEndpointLockTime), try XCTUnwrap(timeline.closeEnoughFirstTime))
        XCTAssertGreaterThan(try XCTUnwrap(timeline.closeEnoughFirstTime), 1.3)
        XCTAssertLessThan(try XCTUnwrap(timeline.closeEnoughFirstTime), 1.5)
    }
}
