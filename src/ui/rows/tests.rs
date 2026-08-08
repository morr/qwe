use super::*;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Sample {
    A,
    B,
    C,
}

#[test]
fn next_in_cycles_and_wraps() {
    let all = [Sample::A, Sample::B, Sample::C];
    assert_eq!(next_in(&all, Sample::A), Sample::B);
    assert_eq!(next_in(&all, Sample::B), Sample::C);
    assert_eq!(next_in(&all, Sample::C), Sample::A);
}

/// Значение не из набора не должно ронять кнопку: до вынесения в общий модуль
/// это поведение было записано в двух копиях и не проверено ни в одной.
#[test]
fn next_in_falls_back_to_the_first_for_an_unknown_value() {
    assert_eq!(next_in(&[Sample::B, Sample::C], Sample::A), Sample::B);
}

#[test]
fn on_off_reads_as_the_panels_expect() {
    assert_eq!(on_off(true), "On");
    assert_eq!(on_off(false), "Off");
}
