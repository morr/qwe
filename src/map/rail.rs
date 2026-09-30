//! Железнодорожный путь — не картографический символ, а сама конструкция:
//! балласт с плечом, шпалы поперёк него и две стальные нитки по колее. Раньше
//! путь рисовался как в osm-carto (тёмная лента и белая штриховка поверх) и на
//! приближении оставался знаком на карте, тогда как всё вокруг — дома с
//! тенями, кроны, бордюры мостов — рисуется предметно.
//!
//! Предметность стоит геометрии: шпала каждые 65 см по всем путям города
//! осмысленна только там, где шпалу видно. Поэтому путь, как и трамвай
//! (`map/tram.rs`), пересобирается по ступеням зума ([`RAIL_LODS`]), и ступени
//! меняют не размер одного и того же рисунка, а сам рисунок:
//!
//! - вблизи — настоящий путь: шпалы с шагом 65 см и нитки по колее 1.5 м;
//! - на среднем плане нитки сходятся в одну полосу и убираются, шпалы редеют
//!   до штриховки поперёк балласта;
//! - на общем плане возвращается пунктирный знак osm-carto — штрих темнее
//!   балласта (см. [`RailPalette::dash`]), потому что серая полоса без него
//!   читается как ещё одна улица.
//!
//! Слои — три меша (`Z_RAIL` / `Z_RAIL_TIE` / `Z_RAIL_STEEL`), а не один: в
//! общем меше компланарная геометрия z-файтит, а шпала одного пути обязана
//! лежать поверх балласта соседнего — иначе развязка расслаивается на
//! отдельные ways.
//!
//! Стиль дорог (`RoadStyle`) пути не касается — как и у трамвая: на ступенях
//! LOD ширины считаются от зума, и ручка сглаживания, меняющая осевую, гоняла
//! бы путь относительно неподвижного балласта. Навмеша путь не касается тоже —
//! люди ходят через рельсы как по земле.

use std::borrow::Cow;

use bevy::prelude::*;

use crate::map::grid::Grid;
use crate::map::meshing::{MeshBuilder, RibbonCap, RibbonJoin};
use crate::map::osm::{MapData, RailKind, RailLine, RoadClass, RoadLine};
use crate::map::smooth::{Smoothing, smooth_path};
use crate::map::surface::{self, LayerCost, LayerMaterials, LayerMesh, MaterialSpec, spawn_layers};
use crate::map::zoom::{ZoomBucket, ZoomLods};
use crate::prefs::retuned;
use crate::settings::{
    Z_RAIL, Z_RAIL_BRIDGE, Z_RAIL_BRIDGE_STEEL, Z_RAIL_BRIDGE_TIE, Z_RAIL_STEEL, Z_RAIL_TIE,
};

/// Цвета одного вида пути. Действующий путь — щебень, креозотная шпала и
/// накатанная до блеска головка рельса; заброшенный — тот же путь, заросший:
/// балласт уходит в травяной оттенок, шпала седеет, нитка ржавеет.
pub struct RailPalette {
    /// Плечо балластной призмы — откос по краю, темнее верха.
    pub shoulder: Color,
    pub ballast: Color,
    pub tie: Color,
    pub steel: Color,
    /// Штриховка дальних ступеней поверх балласта. Знак пришёл из osm-carto и
    /// был там белым; на первом же закадровом снимке белая лесенка через весь
    /// станционный парк оказалась самым картографическим, что есть в кадре.
    /// Поэтому штрих теперь **темнее балласта**: шпал на этих ступенях уже нет
    /// ([`RAIL_LODS`]), и сверху путь читается серой лентой с тёмным пунктиром
    /// по осевой — меткой самого пути, а не белой лентой.
    ///
    /// Значение — **ровно [`RailPalette::tie`] своей палитры**, и это не
    /// совпадение: на ступени 2 поперёк того же балласта лежат эти самые
    /// шпалы, так что на пороге зума метка не светлеет скачком, а продолжает
    /// их собой. Поэтому обе стоят на одной константе палитры (`*_TIE`), и
    /// разойтись им нельзя по построению. Запаса на расхождение и нет:
    /// на последней ступени штрих шириной ровно в пиксель ([`RAIL_LODS`]),
    /// и весь знак держится на разнице тона с балластом.
    pub dash: Color,
}

/// Шпала действующего пути — она же его дальний штрих, см. [`RailPalette::dash`].
const ACTIVE_TIE: Color = Color::srgb(0.243, 0.196, 0.157);

const ACTIVE: RailPalette = RailPalette {
    shoulder: Color::srgb(0.376, 0.357, 0.325),
    ballast: Color::srgb(0.478, 0.455, 0.427),
    tie: ACTIVE_TIE,
    steel: Color::srgb(0.792, 0.804, 0.827),
    dash: ACTIVE_TIE,
};

/// Шпала заброшенного пути — она же его дальний штрих, см. [`RailPalette::dash`].
const DISUSED_TIE: Color = Color::srgb(0.400, 0.361, 0.302);

const DISUSED: RailPalette = RailPalette {
    shoulder: Color::srgb(0.451, 0.451, 0.400),
    ballast: Color::srgb(0.549, 0.545, 0.482),
    tie: DISUSED_TIE,
    steel: Color::srgb(0.545, 0.400, 0.322),
    dash: DISUSED_TIE,
};

/// Ширина плеча как доля ширины балласта. Призма шире своего верха: у
/// однопутного участка верх ~4.5 м при подошве около 6 м, и именно откос
/// отделяет путь от земли, по которой он идёт. Подошву ([`deck_width`])
/// читает и зона запрета машин (`cars::rails`): кузов держится от кромки плеча.
pub(crate) const SHOULDER_SCALE: f32 = 1.22;

/// Ширина, зажимающая срез Chaikin у пути. Как у трамвая — константа, а не
/// ширина ступени: осевая обязана остаться одной и той же на всех ступенях,
/// иначе путь ёрзает относительно самого себя при переходе через порог зума.
const RAIL_SMOOTH_WIDTH: f32 = 5.0;

/// Осевая пути так, как её рисует этот слой. Её же берёт настил путепровода
/// (`roads/bridges.rs`): мост обязан лечь ровно под свой путь.
pub(crate) fn track_centerline(rail: &RailLine) -> Cow<'_, [Vec2]> {
    smooth_path(&rail.points, RAIL_SMOOTH_WIDTH, RAIL_SMOOTHING)
}

/// Ширина настила под путём — подошва балластной призмы: на путепроводе
/// откоса нет, и на его месте лежит плита моста (`roads/bridges.rs`); на
/// переезде — светлый настил поверх щебня ([`mesh_level_crossings`]). От её
/// кромки держатся и машины (`cars::rails::RailKeepout`).
pub(crate) fn deck_width(rail: &RailLine) -> f32 {
    rail.width * SHOULDER_SCALE
}

/// Цвет настила переезда — светлые бетонные плиты между рельсами и вокруг
/// них, светлее асфальта, на который они выходят: так переезд и читается
/// сверху.
const CROSSING_COLOR: Color = Color::srgb(0.69, 0.68, 0.65);

/// Под каким наименьшим углом путь ещё **пересекает** дорогу, синус: при
/// меньшем он идёт вдоль неё (подъездной путь по улице), и «переезд» по
/// формуле растянулся бы на сотни метров асфальта.
const CROSSING_MIN_SIN: f32 = 0.34;

/// Ячейка сетки звеньев пути для поиска переездов, м.
const CROSSING_CELL: f32 = 32.0;

/// Стиль зафиксирован, без ручек панели (см. модульную прозу): осевая слегка
/// сглажена, стыки круглые — ломаная OSM на повороте даёт балласту заметный
/// угол, а шпалы на нём разъезжаются веером.
const RAIL_JOIN: RibbonJoin = RibbonJoin::Round;
const RAIL_CAP: RibbonCap = RibbonCap::Round;
const RAIL_SMOOTHING: Smoothing = Smoothing::Light;

/// Шпалы одной ступени: длина поперёк пути как доля ширины балласта (у
/// узкоколейки балласт уже, и шпала обязана быть короче), толщина и шаг, м.
pub struct RailTieLod {
    pub length_scale: f32,
    pub thickness: f32,
    pub spacing: f32,
}

/// Нитки одной ступени: колея и ширина самой нитки, м. Колея — абсолютная,
/// одна на все виды пути (1.5 м — стандартные 1520 мм; light_rail, метро и
/// заброшенный путь физически той же колеи, у него лишь заросший балласт), а
/// не доля балласта: на 4- и 3.5-метровом балласте доля 0.30 сводила нитки на
/// ступени 1 до 3.6 и 3.0 px, ниже порога, на котором они читаются порознь.
pub struct RailSteelLod {
    pub gauge: f32,
    pub width: f32,
}

/// Штриховка дальних ступеней: длина штриха, пропуска и ширина как доля
/// балласта. Рисунок — из osm-carto, которым путь рисовался целиком до LOD;
/// цвет свой, см. [`RailPalette::dash`].
pub struct RailDashLod {
    pub length: f32,
    pub gap: f32,
    pub width_scale: f32,
}

/// Ступень зум-LOD пути: до какого зума действует и чем на ней путь нарисован.
/// Зум — мировых метров на логический пиксель (`PanCamera`).
pub struct RailLod {
    /// Верхняя (исключающая) граница ступени.
    pub max_zoom: f32,
    /// Нижняя граница ширины балласта, м: ширина из OSM (5 м у магистрального
    /// пути, 4 у узкоколейки и метро) на общем плане города уходит в
    /// пиксель-другой, и путь пропадает раньше улиц, которые он пересекает.
    pub min_bed: f32,
    /// `None` — на этой ступени шпал нет.
    pub tie: Option<RailTieLod>,
    /// `None` — нитки сошлись бы в одну полосу и не рисуются.
    pub steel: Option<RailSteelLod>,
    /// `None` — путь читается своей конструкцией, знак не нужен.
    pub dash: Option<RailDashLod>,
}

/// Ступени зум-LOD. Числа выведены из экранного размера на **худшем** краю
/// ступени (у верхней границы зума): шаг шпал нигде не падает ниже ~6 px, ни
/// шпала, ни нитка, ни штрих — ниже ~1 px. Отсюда и три разных рисунка: с
/// 0.26 м/px колея в 1.5 м это уже 6 px, две нитки по одному пикселю в них не
/// разделяются, а с 0.65 м/px шпалы приходится ставить реже, чем их видно как
/// шпалы, — дальше честнее знак, чем стёршаяся конструкция.
///
/// Второе число, которое держится поперёк ступеней, — **доля шпалы в шаге**,
/// около 40%: у настоящего пути это 0.26 м на 0.65, и стоит ей упасть, как
/// шпалы перестают быть текстурой и становятся редкими метками, а две белые
/// нитки перевешивают их и путь читается лестницей, а не путём.
pub const RAIL_LODS: [RailLod; 5] = [
    RailLod {
        max_zoom: 0.10,
        min_bed: 0.0,
        // настоящая геометрия: шпала 2.6 × 0.26 м с шагом 65 см
        tie: Some(RailTieLod {
            length_scale: 0.52,
            thickness: 0.26,
            spacing: 0.65,
        }),
        steel: Some(RailSteelLod {
            gauge: 1.5,
            width: 0.12,
        }),
        dash: None,
    },
    RailLod {
        max_zoom: 0.26,
        min_bed: 0.0,
        tie: Some(RailTieLod {
            length_scale: 0.52,
            thickness: 0.64,
            spacing: 1.6,
        }),
        steel: Some(RailSteelLod {
            gauge: 1.5,
            width: 0.26,
        }),
        dash: None,
    },
    RailLod {
        max_zoom: 0.65,
        min_bed: 0.0,
        tie: Some(RailTieLod {
            length_scale: 0.56,
            thickness: 1.5,
            spacing: 4.0,
        }),
        steel: None,
        dash: None,
    },
    RailLod {
        max_zoom: 1.6,
        min_bed: 5.0,
        tie: None,
        steel: None,
        dash: Some(RailDashLod {
            length: 9.0,
            gap: 9.0,
            width_scale: 0.5,
        }),
    },
    RailLod {
        max_zoom: f32::INFINITY,
        min_bed: 9.0,
        tie: None,
        steel: None,
        dash: Some(RailDashLod {
            length: 22.0,
            gap: 22.0,
            width_scale: 0.5,
        }),
    },
];

/// [`RAIL_LODS`] как таблица ступеней зум-LOD (`map/zoom.rs`). Своя, а не
/// трамвайная: у пути и порогов больше, и смысл у них другой. Пустой enum —
/// тип-маркер, значений у него не бывает.
pub enum RailLods {}

impl ZoomLods for RailLods {
    fn max_zooms() -> impl Iterator<Item = f32> {
        RAIL_LODS.into_iter().map(|lod| lod.max_zoom)
    }
}

/// Текущая ступень [`RAIL_LODS`]; пересечение порога пересобирает рельсовые
/// слои ([`rebuild_rails`]).
pub type RailZoomBucket = ZoomBucket<RailLods>;

/// Рельсовый слой карты — чтобы пересборка знала, что деспавнить.
///
/// `Copy` — метку получает каждый из трёх слоёв, а сама она пуста.
#[derive(Component, Clone, Copy)]
pub struct RailLayerTag;

/// Один путь, приведённый к тому, что нужно рисованию: сглаженная осевая,
/// ширина балласта на этой ступени и палитра своего вида. Считается один раз,
/// потому что проходов по путям несколько — слои обязаны собираться целиком,
/// а не путь за путём (см. модульную прозу про z-файтинг).
struct Track<'a> {
    points: Cow<'a, [Vec2]>,
    bed: f32,
    palette: &'static RailPalette,
    /// Путь на путепроводе: свои слои над настилом моста, без откоса.
    bridge: bool,
}

/// Что вышло из сборки путей — значением, а не только строкой в логе.
///
/// `tracks` — сколько путей действительно нарисовано: трамвайные сюда не
/// попадают, у них свой модуль; `bridges` — сколько из них на путепроводе.
/// `elapsed` меряется внутри сборки, потому что время тратится там; печатает
/// его адаптер.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RailReport {
    pub tracks: usize,
    pub bridges: usize,
    pub bucket: usize,
    pub vertices: usize,
    pub elapsed: std::time::Duration,
}

impl std::fmt::Display for RailReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            tracks,
            bridges,
            bucket,
            vertices,
            elapsed,
        } = self;
        write!(
            f,
            "rail meshing: {tracks} tracks ({bridges} on bridges), {vertices} verts in \
             {elapsed:?} (bucket {bucket})"
        )
    }
}

/// Рельсовые слои текущей ступени зума, снизу вверх: балласт, шпалы, сталь —
/// и те же три слоя путей на путепроводах (`RailLine::bridge`), над настилом
/// моста. Сам настил, парапет и тень путепровода кладёт слой мостов улиц
/// (`roads/bridges.rs`): тень там объединяется с тенями мостов улиц, и
/// пересборку по солнцу слой дорог уже умеет. Настил переездов — тоже слой
/// дорог ([`mesh_level_crossings`]): ему нужна ось улицы так, как она
/// нарисована.
///
/// **Чистая функция и единственная дверь в слой.** Ни `Commands`, ни `Assets`:
/// её зовёт и игра (через [`rebuild_rails`]), и тест. Три меша, а не один, по
/// той же причине, по которой они три и в мире: копланарная геометрия
/// z-файтит, и шпала обязана лежать выше **любого** балласта, иначе развязка
/// нескольких путей расслаивается.
pub fn mesh_rails(bucket: RailZoomBucket, rails: &[RailLine]) -> (Vec<LayerMesh>, RailReport) {
    let started = std::time::Instant::now();
    let lod = &RAIL_LODS[bucket.index];

    let tracks: Vec<Track> = rails
        .iter()
        .filter_map(|rail| {
            let palette = match rail.kind {
                // трамвай — свой меш со своим стилем и зум-LOD (`map/tram.rs`)
                RailKind::Tram => return None,
                RailKind::Active => &ACTIVE,
                RailKind::Disused => &DISUSED,
            };
            let bed = rail.width.max(lod.min_bed);
            Some(Track {
                points: track_centerline(rail),
                // на мосту балласт не шире плиты под ним: дальние ступени
                // раздувают его до пикселя, и он свисал бы за парапет
                bed: if rail.bridge {
                    bed.min(deck_width(rail))
                } else {
                    bed
                },
                palette,
                bridge: rail.bridge,
            })
        })
        .collect();
    let ground: Vec<&Track> = tracks.iter().filter(|track| !track.bridge).collect();
    let decks: Vec<&Track> = tracks.iter().filter(|track| track.bridge).collect();

    let [ballast, ties, steel] = push_tracks(&ground, lod, true);
    let [bridge_ballast, bridge_ties, bridge_steel] = push_tracks(&decks, lod, false);

    let layers: Vec<LayerMesh> = [
        (ballast, Z_RAIL, "rail_ballast"),
        (ties, Z_RAIL_TIE, "rail_ties"),
        (steel, Z_RAIL_STEEL, "rail_steel"),
        (bridge_ballast, Z_RAIL_BRIDGE, "rail_bridge_ballast"),
        (bridge_ties, Z_RAIL_BRIDGE_TIE, "rail_bridge_ties"),
        (bridge_steel, Z_RAIL_BRIDGE_STEEL, "rail_bridge_steel"),
    ]
    .into_iter()
    // вершинные цвета — материал белый и плоский: фактура поверхностей пути
    // ни к чему, он и так весь из щебня, шпал и стали
    .map(|(builder, z, name)| LayerMesh::new(builder, z, name, MaterialSpec::Flat))
    .collect();
    let report = RailReport {
        tracks: tracks.len(),
        bridges: decks.len(),
        bucket: bucket.index,
        vertices: layers
            .iter()
            .map(|layer| layer.builder.vertex_count())
            .sum(),
        elapsed: started.elapsed(),
    };
    (layers, report)
}

/// Балласт, шпалы (или штрих) и сталь одной группы путей — трёх мешей, а не
/// одного (см. модульную прозу). `shoulder` — класть ли откос призмы: у пути
/// на путепроводе его нет, балласт лежит в корыте моста.
fn push_tracks(tracks: &[&Track], lod: &RailLod, shoulder: bool) -> [MeshBuilder; 3] {
    let mut ballast = MeshBuilder::default();
    if shoulder {
        for track in tracks {
            ballast.push_ribbon(
                &track.points,
                false,
                track.bed * SHOULDER_SCALE,
                track.palette.shoulder.to_linear(),
                RAIL_JOIN,
                RAIL_CAP,
            );
        }
    }
    // верх призмы — вторым проходом: плечо соседнего пути не должно ложиться
    // на балласт этого, иначе развязка расчерчивается тёмными полосами
    for track in tracks {
        ballast.push_ribbon(
            &track.points,
            false,
            track.bed,
            track.palette.ballast.to_linear(),
            RAIL_JOIN,
            RAIL_CAP,
        );
    }

    let mut ties = MeshBuilder::default();
    for track in tracks {
        if let Some(tie) = &lod.tie {
            ties.push_ticks(
                &track.points,
                track.bed * tie.length_scale,
                tie.thickness,
                tie.spacing,
                track.palette.tie.to_linear(),
            );
        }
        if let Some(dash) = &lod.dash {
            ties.push_dashes(
                &track.points,
                track.bed * dash.width_scale,
                dash.length,
                dash.gap,
                track.palette.dash.to_linear(),
                RibbonJoin::Round,
            );
        }
    }

    let mut steel = MeshBuilder::default();
    for track in tracks {
        if let Some(rails_lod) = &lod.steel {
            steel.push_rails(
                &track.points,
                rails_lod.gauge,
                rails_lod.width,
                track.palette.steel.to_linear(),
                RibbonJoin::Round,
            );
        }
    }
    [ballast, ties, steel]
}

/// Дорога, которую путь может пересечь в одном уровне: проезжая часть любого
/// класса — улица или проезд, — не на мосту и не в арке. Мост над путём
/// путь не пересекает, а путепровод над дорогой сюда не попадает вовсе:
/// переезды ищутся только у наземных путей.
pub(crate) fn crossable(road: &RoadLine) -> bool {
    road.class == RoadClass::Street && !road.bridge && !road.passage
}

/// Улица, которую путь может пересечь в одном уровне ([`crossable`]), — так,
/// как она **нарисована**: ось ленты и полоса `[слева, справа]` по ходу
/// точек (полпроезжей части плюс тротуар, если он рисуется с этой стороны).
/// Не осевая OSM: ось улицы сглажена и сдвинута допуском кривой
/// (`RoadShape::curve_tolerance`, до 3 м), и настил по осевой OSM съезжал с
/// асфальта на обочину (Тула, 2612 3130).
pub(crate) struct CrossedStreet<'a> {
    pub axis: &'a [Vec2],
    pub reach: [f32; 2],
}

/// **Переезды в одном уровне**: где осевая наземного пути пересекает осевую
/// проезжей части, щебень и шпалы на ширине дороги с её тротуарами
/// закрываются светлым настилом, а нитки идут поверх него, как трамвайные по
/// асфальту. За кромкой дороги путь снова обычный — балласт и шпалы.
///
/// Тега для этого нет: `railway=level_crossing` в запросе не стоит, и даже
/// скачай его — в кеше v15 у Тулы такой узел один, у Орла ни одного. Поэтому
/// переезд ищется **геометрией**: пересечение звена пути со звеном дороги.
/// Слой дороги (`layer`) не читается — путь на мосту уже отсеян
/// (`RailLine::bridge`), дорога на мосту — тоже ([`crossable`]).
///
/// Настил — **параллелограмм** пересечения двух прямых полос: вдоль пути —
/// подошва балласта ([`deck_width`]), поперёк — проезжая часть и тротуары по
/// обе стороны, каждый своей ширины ([`CrossedStreet`]). Его торцы идут по
/// кромкам дороги, а не поперёк пути, поэтому на косом переезде щебень не
/// выглядывает на асфальт треугольником. При угле меньше
/// [`CROSSING_MIN_SIN`] путь идёт вдоль дороги, а не через неё, и настила
/// нет.
///
/// Кладёт его слой дорог (`map::roads`, слой `rail_crossings` на
/// `Z_RAIL_CROSSING`): только там есть ось улицы так, как она нарисована,
/// и пересборка по ручкам формы улиц. Путь от этого не зависит — ось пути
/// та же [`track_centerline`], ширина — от OSM, не от ступени зума.
///
/// Возвращает меш и число переездов (пар звеньев).
pub(crate) fn mesh_level_crossings(
    rails: &[RailLine],
    streets: &[CrossedStreet],
) -> (MeshBuilder, usize) {
    let mut builder = MeshBuilder::default();
    let tracks: Vec<(Cow<[Vec2]>, f32)> = rails
        .iter()
        .filter(|rail| rail.kind != RailKind::Tram && !rail.bridge)
        .map(|rail| (track_centerline(rail), deck_width(rail) / 2.0))
        .collect();
    if tracks.is_empty() {
        return (builder, 0);
    }
    let mut links: Grid<(usize, usize)> = Grid::new(CROSSING_CELL);
    for (track, (points, _)) in tracks.iter().enumerate() {
        for (link, pair) in points.windows(2).enumerate() {
            links.insert_segment(pair[0], pair[1], 0.0, (track, link));
        }
    }
    let color = CROSSING_COLOR.to_linear();
    let mut count = 0;
    for street in streets {
        for pair in street.axis.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            for (track, link) in links.near(from.min(to), from.max(to)) {
                let (points, half) = &tracks[track];
                let Some(quad) = crossing_deck(
                    [points[link], points[link + 1]],
                    *half,
                    [from, to],
                    street.reach,
                ) else {
                    continue;
                };
                builder.push_polygon(&quad, &[], color);
                count += 1;
            }
        }
    }
    (builder, count)
}

/// Параллелограмм настила одного переезда (см. [`mesh_level_crossings`]): звено
/// пути с полушириной `half`, звено дороги с полосой `reach` `[слева,
/// справа]`. `None` — звенья не пересекаются или идут почти вдоль.
fn crossing_deck(
    track: [Vec2; 2],
    half: f32,
    road: [Vec2; 2],
    reach: [f32; 2],
) -> Option<[Vec2; 4]> {
    let along = (track[1] - track[0]).try_normalize()?;
    let across = (road[1] - road[0]).try_normalize()?;
    let sine = along.perp_dot(across);
    if sine.abs() < CROSSING_MIN_SIN {
        return None;
    }
    // пересечение осевых: параметр на каждом звене — внутри [0, 1]
    let span = track[1] - track[0];
    let reach_road = road[1] - road[0];
    let denom = span.perp_dot(reach_road);
    let offset = road[0] - track[0];
    let t = offset.perp_dot(reach_road) / denom;
    let u = offset.perp_dot(span) / denom;
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    let center = track[0] + span * t;
    // угол настила — точка, где кромка пути (±half по нормали пути)
    // встречает кромку дороги (+left / −right по нормали дороги)
    let normals = Mat2::from_cols(along.perp(), across.perp()).transpose();
    let inverse = normals.inverse();
    let corner = |side: f32, edge: f32| center + inverse * Vec2::new(side * half, edge);
    let [left, right] = reach;
    Some([
        corner(-1.0, -right),
        corner(1.0, -right),
        corner(1.0, left),
        corner(-1.0, left),
    ])
}

/// Офлайн-замер путевых слоёв — **по строке на ступень зума**, а не одной
/// строкой: у рельсов ступени отличаются не размером, а тем, что нарисовано,
/// и дальняя стоит 45 к вершин против 673 к у ближней. Одно число здесь было
/// бы числом ни о чём.
///
/// Своей сборки у замера нет: он зовёт тот же [`mesh_rails`], что и игра.
pub fn measure_rails(rails: &[RailLine]) -> Vec<(usize, Vec<LayerCost>)> {
    (0..RAIL_LODS.len())
        .map(|index| {
            let (layers, report) = mesh_rails(RailZoomBucket::at(index), rails);
            (index, surface::layer_costs(&layers, report.elapsed))
        })
        .collect()
}

/// Когда пересобирать путевые слои. Только ступень зума: у пути нет ни ручек
/// стиля (`RoadStyle` его не касается), ни теней, так что солнце ему
/// безразлично — единственное, что меняет рисунок, это порог зума.
///
/// **Условие одно, регистрация одна** (см. `roads::rebuilds_on`).
pub fn rebuilds_on() -> impl SystemCondition<()> {
    // через `into_system`, потому что у голой функции-условия свой маркер
    // типа; у остальных слоёв его стирает `or_else`, а здесь складывать нечего
    IntoSystem::into_system(retuned::<RailZoomBucket>)
}

/// Пересборка рельсовых слоёв при смене ступени зума — дорожные и трамвайный
/// слои не трогаются.
pub fn rebuild_rails(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: LayerMaterials,
    bucket: Res<RailZoomBucket>,
    map: Res<MapData>,
    existing: Query<Entity, With<RailLayerTag>>,
) {
    for entity in &existing {
        commands.entity(entity).despawn();
    }
    let (layers, report) = mesh_rails(*bucket, &map.rails);
    spawn_layers(&mut commands, &mut meshes, &materials, layers, RailLayerTag);
    info!("{report}");
}

#[cfg(test)]
mod tests;
