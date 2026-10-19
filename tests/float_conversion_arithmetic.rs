//! `u128` decimals converted to `f32`: a mantissa near `u128::MAX` is infinite as an `f32` even
//! when the scaled decimal is finite (or rounds to zero), so the scale must be applied first.
use primitive_fixed_point_decimal::{ConstScaleFpdec as C, OobScaleFpdec as D};

/// `u128::MAX` at scale `S` as `f32`, checked against the bits of the correctly rounded value.
fn check<const S: i32>(bits: u32) {
    let c = C::<u128, S>::MAX;
    let d = D::<u128>::MAX;
    assert_eq!(f32::from(c).to_bits(), bits, "const u128::MAX scale {S}");
    assert_eq!(d.to_f32(S).to_bits(), bits, "oob u128::MAX scale {S}");
}

#[test]
fn u128_mantissa_overflow_must_not_destroy_representable_f32_decimal() {
    check::<1>(2110573773);
    check::<2>(2082723594);
    check::<18>(1637061359);
    check::<30>(1302479456);
    check::<38>(1079625693);
    check::<39>(1051605374);
    check::<40>(1024155953);
    check::<45>(884387853);
    check::<77>(2428336);
    check::<83>(2);
    check::<84>(0);
    check::<100>(0);
    check::<{ i32::MAX }>(0);

    // at scale 0 or below the value really is beyond `f32::MAX`
    assert!(f32::from(C::<u128, 0>::MAX).is_infinite());
    assert!(D::<u128>::MAX.to_f32(0).is_infinite());
    assert!(f32::from(C::<u128, -1>::MAX).is_infinite());
    assert!(D::<u128>::MAX.to_f32(-1).is_infinite());
}
