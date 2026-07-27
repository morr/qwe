//! Раскладка города, забитая вручную по мотивам `assets/map.png` (Тула,
//! Первомайская / парк). Все координаты — метры, начало — юго-западный угол.
//!
//! Названия улиц — ориентиры с реальной карты, геометрия схематична.

use bevy::prelude::*;

use crate::settings::MAP_SIZE;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoadKind {
    /// Городская улица, белая полоса.
    Street,
    /// Парковая аллея, песчаная узкая полоса.
    Alley,
}

pub struct Road {
    pub from: Vec2,
    pub to: Vec2,
    pub width: f32,
    pub kind: RoadKind,
}

const fn street(from: (f32, f32), to: (f32, f32), width: f32) -> Road {
    Road {
        from: Vec2::new(from.0, from.1),
        to: Vec2::new(to.0, to.1),
        width,
        kind: RoadKind::Street,
    }
}

const fn alley(from: (f32, f32), to: (f32, f32)) -> Road {
    Road {
        from: Vec2::new(from.0, from.1),
        to: Vec2::new(to.0, to.1),
        width: 4.0,
        kind: RoadKind::Alley,
    }
}

/// Центр парковой площади, от неё расходятся аллеи.
pub const PARK_PLAZA: Vec2 = Vec2::new(650.0, 120.0);
pub const PARK_PLAZA_RADIUS: f32 = 25.0;

pub const ROADS: &[Road] = &[
    // Демонстрации — северная магистраль
    street((0.0, 830.0), (1200.0, 830.0), 16.0),
    // Льва Толстого
    street((500.0, 640.0), (1200.0, 640.0), 10.0),
    // Первомайская — диагональ, граница парка и города
    street((240.0, 560.0), (620.0, 400.0), 18.0),
    street((620.0, 400.0), (1120.0, 120.0), 18.0),
    // Дмитрия Ульянова
    street((300.0, 900.0), (260.0, 560.0), 14.0),
    // Вересаева
    street((560.0, 900.0), (560.0, 410.0), 10.0),
    // Фрунзе
    street((800.0, 900.0), (800.0, 330.0), 12.0),
    // Хворостухина
    street((1060.0, 900.0), (1060.0, 140.0), 10.0),
    // Софьи Перовской
    street((900.0, 520.0), (1200.0, 520.0), 8.0),
    // Парковые аллеи от площади
    alley((650.0, 120.0), (618.0, 392.0)),
    alley((650.0, 120.0), (420.0, 20.0)),
    alley((650.0, 120.0), (880.0, 20.0)),
    alley((650.0, 120.0), (360.0, 240.0)),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BuildingKind {
    /// Многоэтажка-«пластина», 12 м в глубину.
    Slab,
    /// Частный дом 8 × 10 м.
    House,
}

#[derive(Clone, Copy)]
pub struct Building {
    pub min: Vec2,
    pub size: Vec2,
    pub kind: BuildingKind,
}

impl Building {
    pub fn center(&self) -> Vec2 {
        self.min + self.size / 2.0
    }

    pub fn max(&self) -> Vec2 {
        self.min + self.size
    }
}

const fn slab(x: f32, y: f32, w: f32, h: f32) -> Building {
    Building {
        min: Vec2::new(x, y),
        size: Vec2::new(w, h),
        kind: BuildingKind::Slab,
    }
}

/// Многоэтажки по кварталам (между улицами из `ROADS`).
pub const SLABS: &[Building] = &[
    // Квартал Ульянова — Вересаева
    slab(330.0, 660.0, 12.0, 150.0),
    slab(370.0, 795.0, 170.0, 12.0),
    slab(370.0, 655.0, 160.0, 12.0),
    slab(430.0, 700.0, 12.0, 80.0),
    slab(480.0, 740.0, 60.0, 12.0),
    // Квартал Вересаева — Фрунзе, север
    slab(585.0, 795.0, 190.0, 12.0),
    slab(585.0, 655.0, 12.0, 130.0),
    slab(635.0, 700.0, 120.0, 12.0),
    slab(765.0, 700.0, 12.0, 90.0),
    slab(635.0, 655.0, 110.0, 12.0),
    // Квартал Фрунзе — Хворостухина, север
    slab(830.0, 795.0, 200.0, 12.0),
    slab(830.0, 655.0, 12.0, 120.0),
    slab(885.0, 700.0, 140.0, 12.0),
    slab(1030.0, 655.0, 12.0, 120.0),
    // Восточный квартал, север
    slab(1085.0, 795.0, 105.0, 12.0),
    slab(1085.0, 655.0, 12.0, 120.0),
    slab(1130.0, 700.0, 60.0, 12.0),
    // Квартал Вересаева — Фрунзе, юг (над Первомайской)
    slab(585.0, 600.0, 180.0, 12.0),
    slab(585.0, 455.0, 12.0, 135.0),
    slab(640.0, 540.0, 120.0, 12.0),
    slab(700.0, 470.0, 80.0, 12.0),
    // Квартал Фрунзе — Хворостухина, юг
    slab(830.0, 600.0, 200.0, 12.0),
    slab(885.0, 545.0, 140.0, 12.0),
    slab(830.0, 430.0, 12.0, 80.0),
    slab(885.0, 470.0, 120.0, 12.0),
    slab(1015.0, 430.0, 12.0, 80.0),
    // Восточный квартал, юг
    slab(1085.0, 560.0, 105.0, 12.0),
    slab(1085.0, 430.0, 12.0, 120.0),
    slab(1130.0, 470.0, 60.0, 12.0),
    slab(1085.0, 250.0, 12.0, 140.0),
    slab(1120.0, 330.0, 70.0, 12.0),
    slab(1120.0, 180.0, 70.0, 12.0),
];

/// Зона частной застройки: сетка домов 8 × 10 с шагом.
struct HouseZone {
    origin: Vec2,
    cols: usize,
    rows: usize,
    step: Vec2,
}

const HOUSE_SIZE: Vec2 = Vec2::new(8.0, 10.0);

const HOUSE_ZONES: &[HouseZone] = &[
    // Северо-запад, за Ульянова
    HouseZone {
        origin: Vec2::new(60.0, 590.0),
        cols: 7,
        rows: 7,
        step: Vec2::new(26.0, 32.0),
    },
    // Юго-запад, за прудом (Одоевская)
    HouseZone {
        origin: Vec2::new(170.0, 310.0),
        cols: 7,
        rows: 6,
        step: Vec2::new(30.0, 34.0),
    },
];

/// Все здания: многоэтажки + частные дома из зон.
pub fn buildings() -> Vec<Building> {
    let mut result: Vec<Building> = SLABS.to_vec();
    for zone in HOUSE_ZONES {
        for col in 0..zone.cols {
            for row in 0..zone.rows {
                result.push(Building {
                    min: zone.origin + zone.step * Vec2::new(col as f32, row as f32),
                    size: HOUSE_SIZE,
                    kind: BuildingKind::House,
                });
            }
        }
    }
    result
}

/// Парк: базовые зелёные прямоугольники (Первомайская рисуется поверх).
pub const PARK_RECTS: &[(Vec2, Vec2)] = &[
    (Vec2::new(240.0, 0.0), Vec2::new(880.0, 390.0)),
    (Vec2::new(240.0, 390.0), Vec2::new(180.0, 90.0)),
];

/// Пруд на западной границе: эллипс (центр, полуоси). Непроходим.
pub const POND_CENTER: Vec2 = Vec2::new(70.0, 250.0);
pub const POND_RADII: Vec2 = Vec2::new(75.0, 190.0);

pub fn is_in_pond(pos: Vec2) -> bool {
    let d = (pos - POND_CENTER) / POND_RADII;
    d.length_squared() <= 1.0
}

pub fn is_in_park(pos: Vec2) -> bool {
    PARK_RECTS
        .iter()
        .any(|(min, size)| pos.cmpge(*min).all() && pos.cmplt(*min + *size).all())
}

/// Кроны деревьев парка: (центр, радиус). Детерминированный LCG — данные
/// одинаковы между запусками, «вручную» задана только зона и плотность.
pub fn trees() -> Vec<(Vec2, f32)> {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((state >> 33) as f32) / (u32::MAX >> 1) as f32
    };

    let mut result = Vec::new();
    while result.len() < 130 {
        let pos = Vec2::new(240.0 + next() * 880.0, next() * 470.0);
        if !is_in_park(pos) {
            continue;
        }
        // не сажаем деревья на площадь
        if (pos - PARK_PLAZA).length() < PARK_PLAZA_RADIUS + 6.0 {
            continue;
        }
        let radius = 2.5 + next() * 1.5;
        result.push((pos, radius));
    }
    result
}

/// Прямоугольник всей карты.
pub const MAP_RECT: (Vec2, Vec2) = (Vec2::ZERO, MAP_SIZE);
