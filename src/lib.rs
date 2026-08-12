pub mod camera;
pub mod city;
pub mod demon;
pub mod determinism;
pub mod dev;
pub mod diagnostics;
pub mod grid;
pub mod human;
pub mod loading;
pub mod map;
pub mod movement;
pub mod navigation;
pub mod portal;
pub mod prefs;
pub mod restart;
pub mod rng;
pub mod settings;
pub mod sim_time;
#[cfg(test)]
// только для юнит-тестов стендов поведения: в боевую библиотеку и
// интеграционные тесты двор не собирается
#[cfg(test)]
pub mod sim_yard;
pub mod spatial;
pub mod telemetry;
pub mod ui;
