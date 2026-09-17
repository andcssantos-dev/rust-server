use aurenfall_core::{QuadrantCoord, QuadrantSeed, QuadrantSizeMm, UniverseSeed, WorldPositionMm};

const PROTOTYPE_QUADRANT_SIZE_MM: i64 = 512_000;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

fn seed_hex(seed: &QuadrantSeed) -> String {
    let mut output = String::with_capacity(64);
    for &byte in seed.as_bytes() {
        output.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[test]
fn vertical_position_does_not_change_discovery_quadrant() -> Result<(), Box<dyn std::error::Error>> {
    let size = QuadrantSizeMm::new(PROTOTYPE_QUADRANT_SIZE_MM)?;
    let ground = WorldPositionMm::new(-512_001, 1_024_000, 0).quadrant_coord(size);
    let underground = WorldPositionMm::new(-512_001, 1_024_000, -5_000_000).quadrant_coord(size);
    let high_altitude = WorldPositionMm::new(-512_001, 1_024_000, 50_000_000).quadrant_coord(size);

    assert_eq!(ground, QuadrantCoord::new(-2, 2));
    assert_eq!(ground, underground);
    assert_eq!(ground, high_altitude);
    Ok(())
}

#[test]
fn quadrant_seed_golden_vectors_are_frozen() {
    let universe = UniverseSeed::from_phrase("aurenfall-golden-quadrant-v1");
    let vectors = [
        (
            QuadrantCoord::new(0, 0),
            "d86b22078ea1fb12f040eaf9b8dc52f2ae915319cc36b5004a833f24ee1be378",
        ),
        (
            QuadrantCoord::new(1, 0),
            "2830739b0cec1dde2395de6abbf2ccc494d73beb22a60c795a5632e44bc88bc1",
        ),
        (
            QuadrantCoord::new(-1, 0),
            "bf2146c6f47527e0e579c5d99e2bf1ba0454559deb6147ed5a29f19cf849f0a3",
        ),
        (
            QuadrantCoord::new(123_456, -987_654),
            "299573b2e66f8db63dec33ba2c260af78e8ff9f0cdf7a5f541a59956d26c4044",
        ),
    ];

    for (coord, expected_hex) in vectors {
        let seed = universe.quadrant_seed(coord, 1);
        let replay = universe.quadrant_seed(coord, 1);
        let actual_hex = seed_hex(&seed);

        assert_eq!(seed, replay);
        assert_eq!(actual_hex, expected_hex);
    }
}
