//! Форма дорог — то, что двигает геометрию, а не краску: ширина полосы,
//! длина клина между сечениями, допуск оси, зазор разделительной и радиус
//! скругления бордюра.
//!
//! Свой ресурс, а не поля [`RoadStyle`](super::RoadStyle): три цены
//! пересборки — три ресурса. `RoadPaintStyle` — юниформы, протяжка не стоит
//! ничего; `RoadStyle` — тумблеры слоёв, одна пересборка на клик; здесь —
//! ползунки, у которых каждое деление пересобирало бы оси, пары, узлы и ряд
//! машин всего города. Поэтому карта следует не за [`RoadShape`], а за
//! [`RoadShapeOnMap`] — копией, которая доезжает до карты после паузы
//! ([`settle_road_shape`]), тем же приёмом, что `SunStyle` → `SunOnMap`.
//!
//! **Ширина полосы — особая**: её читает разбор (`network::sections` —
//! ширина дороги из сечения, а по ширине двигаются дома, дворы и стоянки), так
//! что её смена — перезагрузка мира, как смена города или навтайла
//! (`city::reload_world`). До разбора она доезжает процессной глобалью
//! ([`set_lane_width`]), по той же причине, что солнце и размер навтайла:
//! разбор идёт в потоке загрузки, где ECS нет. Краска и траектории читают ту
//! же глобаль — мир целиком собран с одной шириной полосы.

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::prelude::*;
use bevy::settings::{ReflectSettingsGroup, SettingsGroup};

/// Ширина полосы улицы, м. Проезд уже на столько же, на сколько дефолтная
/// улица шире его дефолта (3.3 против 3.0).
pub const LANE_WIDTH_MIN: f32 = 2.75;
pub const LANE_WIDTH_MAX: f32 = 3.75;
pub const LANE_WIDTH_STEP: f32 = 0.05;
pub const LANE_WIDTH_DEFAULT: f32 = 3.3;
/// Длина клина на метр разницы ширин, м.
pub const TAPER_MIN: f32 = 5.0;
pub const TAPER_MAX: f32 = 20.0;
pub const TAPER_STEP: f32 = 1.0;
/// Насколько ось может уйти от точек OSM, м; `0` — ось как в OSM.
pub const CURVE_TOLERANCE_MIN: f32 = 0.0;
pub const CURVE_TOLERANCE_MAX: f32 = 5.0;
pub const CURVE_TOLERANCE_STEP: f32 = 0.5;
/// Зазор между половинами разделённой улицы, до которого между ними асфальт
/// с двойной сплошной, м; шире — газон с бордюром.
pub const MEDIAN_GAP_MIN: f32 = 1.0;
pub const MEDIAN_GAP_MAX: f32 = 6.0;
pub const MEDIAN_GAP_STEP: f32 = 0.5;
/// Множитель таблицы радиусов бордюра по классам (`roads/corners.rs`).
pub const CORNER_RADIUS_MIN: f32 = 0.5;
pub const CORNER_RADIUS_MAX: f32 = 2.0;
pub const CORNER_RADIUS_STEP: f32 = 0.1;

/// Сколько ползунок должен простоять, чтобы форма доехала до карты, с.
const SHAPE_SETTLE: f32 = 0.35;

/// Форма дорог; панель Roads и BRP, сохраняется между запусками. Значения
/// вне шкалы (старый `settings.toml`, BRP) обрезаются на чтении.
#[derive(Resource, Reflect, SettingsGroup, Clone, Copy, PartialEq, Debug)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "road_shape")]
pub struct RoadShape {
    pub lane_width: f32,
    pub taper: f32,
    pub curve_tolerance: f32,
    pub median_gap: f32,
    pub corner_radius: f32,
}

impl Default for RoadShape {
    fn default() -> Self {
        Self {
            lane_width: LANE_WIDTH_DEFAULT,
            taper: 10.0,
            curve_tolerance: 3.0,
            median_gap: 3.0,
            corner_radius: 1.0,
        }
    }
}

impl RoadShape {
    pub fn lane_width(&self) -> f32 {
        self.lane_width.clamp(LANE_WIDTH_MIN, LANE_WIDTH_MAX)
    }

    pub fn taper(&self) -> f32 {
        self.taper.clamp(TAPER_MIN, TAPER_MAX)
    }

    pub fn curve_tolerance(&self) -> f32 {
        self.curve_tolerance
            .clamp(CURVE_TOLERANCE_MIN, CURVE_TOLERANCE_MAX)
    }

    pub fn median_gap(&self) -> f32 {
        self.median_gap.clamp(MEDIAN_GAP_MIN, MEDIAN_GAP_MAX)
    }

    pub fn corner_radius(&self) -> f32 {
        self.corner_radius
            .clamp(CORNER_RADIUS_MIN, CORNER_RADIUS_MAX)
    }
}

/// Форма, с которой карта **уже собрана**: за ней следуют пересборки дорог и
/// машин (`rebuilds_on`), а не за ползунком.
#[derive(Resource, Reflect, Clone, Copy, PartialEq, Debug, Default)]
#[reflect(Resource, Default)]
pub struct RoadShapeOnMap(pub RoadShape);

/// Стартовая форма: настройки лежат на `RoadShape` ещё до первого
/// расписания, и карта собирается с ними.
pub fn seed_road_shape(shape: Res<RoadShape>, mut on_map: ResMut<RoadShapeOnMap>) {
    on_map.set_if_neq(RoadShapeOnMap(*shape));
}

/// Оседание ползунка: форма доезжает до карты, когда его перестали тянуть.
pub fn settle_road_shape(
    shape: Res<RoadShape>,
    mut on_map: ResMut<RoadShapeOnMap>,
    time: Res<Time<Real>>,
    mut idle: Local<f32>,
) {
    if on_map.0 == *shape {
        *idle = 0.0;
        return;
    }
    if shape.is_changed() {
        *idle = 0.0;
    } else {
        *idle += time.delta_secs();
    }
    if *idle >= SHAPE_SETTLE {
        on_map.0 = *shape;
    }
}

/// Ширина полосы, с которой разобран текущий мир, в битах `f32`.
static LANE_WIDTH_BITS: AtomicU32 = AtomicU32::new(LANE_WIDTH_DEFAULT.to_bits());

/// Ширина полосы улицы, м — та, с которой разобран мир.
pub fn lane_width() -> f32 {
    f32::from_bits(LANE_WIDTH_BITS.load(Ordering::Relaxed))
}

/// Единственная запись — перед разбором (`loading::start_job`, витрина до
/// нарезки окон): после неё мир разбирается и красится с этой шириной.
pub fn set_lane_width(width: f32) {
    LANE_WIDTH_BITS.store(
        width.clamp(LANE_WIDTH_MIN, LANE_WIDTH_MAX).to_bits(),
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_off_the_scale_is_clamped_on_the_read() {
        let shape = RoadShape {
            lane_width: 9.0,
            curve_tolerance: -1.0,
            ..default()
        };
        assert_eq!(shape.lane_width(), LANE_WIDTH_MAX);
        assert_eq!(shape.curve_tolerance(), CURVE_TOLERANCE_MIN);
    }

    /// Ползунок доезжает до карты не сразу: пока его тянут, `RoadShapeOnMap`
    /// стоит на месте, и пересборка не заказана.
    #[test]
    fn the_slider_settles_into_the_map_only_after_a_pause() {
        let mut app = App::new();
        app.init_resource::<RoadShape>()
            .init_resource::<RoadShapeOnMap>()
            .init_resource::<Time<Real>>()
            .add_systems(Update, settle_road_shape);
        app.update();
        app.world_mut().resource_mut::<RoadShape>().taper = 15.0;
        app.update();
        assert_eq!(app.world().resource::<RoadShapeOnMap>().0.taper, 10.0);
        let step = std::time::Duration::from_secs_f32(SHAPE_SETTLE / 2.0);
        for _ in 0..3 {
            app.world_mut()
                .resource_mut::<Time<Real>>()
                .advance_by(step);
            app.update();
        }
        assert_eq!(app.world().resource::<RoadShapeOnMap>().0.taper, 15.0);
    }
}
