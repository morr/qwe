//! Сборка храмов из частей: вера и посев.
//!
//! OSM рисует собор то одним контуром, то общим контуром и частями, и у
//! частей своих тегов почти нет — каждая выбирала бы себе цвет сама. Здесь
//! части находят хозяина по цепочке до верха, берут у него веру и посев, а
//! контуры без назначения, лежащие на храме, становятся его пристройками.

use bevy::math::Vec2;

use crate::map::osm::model::{
    AreaKind, BuildingUse, Faith, MapData, PolyArea, Sacred, SacredForm, point_in_polygon,
    ring_area, ring_bounds, signed_ring_area,
};
use crate::map::seed::seed_from_point;

/// Как далеко от храма ещё стоит его часть — колокольня рядом, придел, м.
const CHURCH_PART_REACH: f32 = 30.0;

/// Контекст прохода веры. Весь след прохода — **одни дома**: что он читает,
/// то и пишет, и других полей у контекста нет. Узкий проход не получает
/// `&mut MapData` целиком: список полей и есть договор с картой — чего здесь
/// нет, того проход не касается, — и тест строит его руками, без карты.
/// `MapData` нужен только мосту [`ResolveFaithsCtx::of`].
pub(super) struct ResolveFaithsCtx<'a> {
    buildings: &'a mut Vec<PolyArea>,
}

impl<'a> ResolveFaithsCtx<'a> {
    /// Собрать контекст с карты — мост между швом и его вызывающим.
    pub(super) fn of(map: &'a mut MapData) -> Self {
        let MapData { buildings, .. } = map;
        Self { buildings }
    }
}

/// Храмы собираются из частей: у каждой — свой **храм** ([`Sacred::complex`])
/// и вера. Возвращает, скольким храмам веру пришлось взять у города.
///
/// OSM рисует собор то одним контуром, то общим контуром и частями
/// (`building:part`, барабаны с `roof:shape=onion`, колокольня рядом), и у
/// частей своих тегов почти нет. В Туле так размечен Успенский собор кремля:
/// вера — на общем контуре, а три главы и колокольня — отдельными контурами
/// без неё, и каждая часть выбирала себе цвет сама — розовый барабан рядом с
/// белым и бирюзовая глава рядом с золотой.
///
/// * **Хозяин части** — самый крупный из храмов крупнее неё, в контуре
///   которого стоит её центр, а если такого нет — ближайший из них не дальше
///   [`CHURCH_PART_REACH`] (колокольня стоит рядом, а не внутри). Храм без
///   хозяина — сам себе храм. Хозяин берётся цепочкой до верха: колокольня,
///   чей ближайший крупный сосед — часть собора, принадлежит собору.
/// * **Посев храма** — от первой вершины верхнего хозяина: им красятся все части.
/// * **Вера** — своя, если размечена; иначе ближайшего по цепочке хозяина с
///   размеченной; иначе большинства храмов
///   города: христианский храм без деноминации в Туле православный, а в Берлине
///   кирха. При равенстве, и в городе без единого размеченного храма, —
///   [`Faith::Western`], самый распространённый в OSM вид церкви.
pub(super) fn resolve_faiths(ctx: ResolveFaithsCtx<'_>) -> usize {
    let buildings: &mut [PolyArea] = ctx.buildings;
    struct Church {
        index: usize,
        faith: Faith,
        area: f32,
        center: Vec2,
        bounds: (Vec2, Vec2),
    }
    let churches: Vec<Church> = buildings
        .iter()
        .enumerate()
        .filter_map(|(index, building)| match building.building_use {
            BuildingUse::Church(sacred) => {
                let bounds = ring_bounds(&building.outer);
                Some(Church {
                    index,
                    faith: sacred.faith,
                    area: ring_area(&building.outer),
                    center: (bounds.0 + bounds.1) * 0.5,
                    bounds,
                })
            }
            _ => None,
        })
        .collect();

    let (orthodox, western) = churches
        .iter()
        .fold((0, 0), |(o, w), church| match church.faith {
            Faith::Orthodox => (o + 1, w),
            Faith::Western => (o, w + 1),
            _ => (o, w),
        });
    let majority = if orthodox > western {
        Faith::Orthodox
    } else {
        Faith::Western
    };

    // хозяин — по снимку до записи: часть не должна стать хозяином по
    // собственной, только что выданной вере
    let hosts: Vec<Option<usize>> = churches
        .iter()
        .map(|part| {
            let larger = churches
                .iter()
                .enumerate()
                .filter(|(_, other)| other.index != part.index && other.area > part.area);
            let inside = larger
                .clone()
                .filter(|(_, other)| {
                    part.center.cmpge(other.bounds.0).all()
                        && part.center.cmple(other.bounds.1).all()
                        && point_in_polygon(part.center, &buildings[other.index].outer)
                })
                .max_by(|a, b| a.1.area.total_cmp(&b.1.area));
            inside.map(|(at, _)| at).or_else(|| {
                larger
                    .map(|(at, other)| {
                        let nearest = part.center.clamp(other.bounds.0, other.bounds.1);
                        (at, nearest.distance(part.center))
                    })
                    .filter(|(_, distance)| *distance <= CHURCH_PART_REACH)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(at, _)| at)
            })
        })
        .collect();

    // хозяин цепочкой: колокольня, ближе всего стоящая к части собора, берёт
    // посев и веру у собора, а не у части. Цепочка конечна — площадь хозяина
    // строго растёт
    let chain = |from: usize| std::iter::successors(Some(from), |&at| hosts[at]);
    let mut guessed = 0;
    for (at, church) in churches.iter().enumerate() {
        let faith = match chain(at)
            .map(|up| churches[up].faith)
            .find(|faith| *faith != Faith::Unknown)
        {
            Some(faith) => faith,
            None => {
                guessed += 1;
                majority
            }
        };
        let root = chain(at).last().unwrap_or(at);
        let anchor = churches[root].index;
        let complex = seed_from_point(buildings[anchor].outer.first().copied().unwrap_or_default());
        if let BuildingUse::Church(sacred) = &mut buildings[church.index].building_use {
            sacred.faith = faith;
            sacred.complex = complex;
        }
    }

    absorb_annexes(buildings);
    guessed
}

/// Во сколько раз пристройка может быть крупнее храма, на котором лежит. Музей
/// оружия в Богоявленском соборе крупнее собора в 1.35 раза — это тот же дом,
/// размеченный вторым контуром; квартал с домовой церковью во дворе крупнее
/// в десятки раз, и храмом он не становится.
const ANNEX_AREA_RATIO: f32 = 2.5;
/// Какую долю своего пятна контур обязан делить с храмом, чтобы стать
/// пристройкой, когда ни один из двух центров не лежит в другом. Алтарная
/// часть Богоявленского собора (20 × 24 м) заходит в собор на половину, а её
/// центр — за его стеной. Дом, лишь примыкающий к храму общей стеной, делит с
/// ним ноль площади и остаётся домом.
const ANNEX_OVERLAP_SHARE: f32 = 0.25;

/// Площадь пересечения двух колец, м².
fn overlap_area(a: &[Vec2], b: &[Vec2]) -> f32 {
    use i_overlay::core::fill_rule::FillRule;
    use i_overlay::core::overlay_rule::OverlayRule;
    use i_overlay::float::single::SingleFloatOverlay;

    let ring = |points: &[Vec2]| -> Vec<[f32; 2]> {
        let mut ring: Vec<[f32; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
        // i_overlay ждёт единую закрутку, OSM её не гарантирует
        if signed_ring_area(points) < 0.0 {
            ring.reverse();
        }
        ring
    };
    let (a, b) = (vec![ring(a)], vec![ring(b)]);
    a.overlay(&b, OverlayRule::Intersect, FillRule::NonZero)
        .iter()
        .flat_map(|shape| shape.iter().enumerate())
        .map(|(index, contour)| {
            let points: Vec<Vec2> = contour.iter().map(|p| Vec2::new(p[0], p[1])).collect();
            // первый контур фигуры — внешний, остальные — дыры
            let area = ring_area(&points);
            if index == 0 { area } else { -area }
        })
        .sum()
}

/// Контуры без назначения, лежащие **на храме**, становятся его пристройками
/// ([`SacredForm::Annex`]): вера и посев храма, храмовые стены и кровля, своих
/// глав нет.
///
/// OSM рисует храм и тем, чем он стал: в Тульском кремле поверх Богоявленского
/// собора лежит контур «музей оружия» (`building=yes`, три этажа), а рядом —
/// пристройка без тегов, и оба рисовались жилыми коробками с окнами, из-за
/// которых торчали главы собора. Признак пристройки — взаимное наложение:
/// центр контура в храме, центр храма в контуре или общая с храмом доля пятна
/// не меньше [`ANNEX_OVERLAP_SHARE`], при площади не больше
/// [`ANNEX_AREA_RATIO`] храма. Храм, к которому она прирастает, — самый крупный
/// из подходящих.
fn absorb_annexes(buildings: &mut [PolyArea]) {
    // Раунды — потому что пристройка прирастает и к пристройке: алтарная часть
    // Богоявленского собора лежит на контуре музея, а с самим собором делит
    // меньше четверти своего пятна. Отдельно стоящий дом ни с чем не
    // перекрывается, так что цепочка сквозь квартал не растёт.
    for _ in 0..ANNEX_ROUNDS {
        if absorb_round(buildings) == 0 {
            break;
        }
    }
}

/// Сколько раундов прирастания пристроек, не больше.
const ANNEX_ROUNDS: usize = 3;

/// Один раунд [`absorb_annexes`]: хозяева — все храмы и пристройки на его
/// начало. Возвращает, сколько контуров стало пристройками.
fn absorb_round(buildings: &mut [PolyArea]) -> usize {
    let candidates: Vec<(usize, (Vec2, Vec2), f32)> = buildings
        .iter()
        .enumerate()
        .filter(|(_, building)| matches!(building.building_use, BuildingUse::Church(_)))
        .map(|(index, building)| {
            (
                index,
                ring_bounds(&building.outer),
                ring_area(&building.outer),
            )
        })
        .collect();
    if candidates.is_empty() {
        return 0;
    }
    let mut absorbed = 0;
    for index in 0..buildings.len() {
        let building = &buildings[index];
        // только контур без назначения (`building=yes`, `building:part`): дом,
        // школа или магазин с названным классом храмом не становятся, как бы
        // ни лежали
        if building.kind != AreaKind::Building || building.building_use != BuildingUse::Other {
            continue;
        }
        let bounds = ring_bounds(&building.outer);
        let center = (bounds.0 + bounds.1) * 0.5;
        let area = ring_area(&building.outer);
        let host = candidates
            .iter()
            .filter(|(church, (lo, hi), church_area)| {
                let overlap = bounds.0.cmple(*hi).all() && bounds.1.cmpge(*lo).all();
                overlap
                    && area <= church_area * ANNEX_AREA_RATIO
                    && (point_in_polygon(center, &buildings[*church].outer)
                        || point_in_polygon((*lo + *hi) * 0.5, &building.outer)
                        || overlap_area(&building.outer, &buildings[*church].outer)
                            >= area * ANNEX_OVERLAP_SHARE)
            })
            .max_by(|a, b| a.2.total_cmp(&b.2));
        let Some(&(church, ..)) = host else {
            continue;
        };
        let BuildingUse::Church(sacred) = buildings[church].building_use else {
            continue;
        };
        buildings[index].building_use = BuildingUse::Church(Sacred {
            form: SacredForm::Annex,
            floor_dm: 0,
            ..sacred
        });
        absorbed += 1;
    }
    absorbed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::osm::fixture::{building, square};
    use crate::settings::MAP_SIZE;

    /// Центр карты, как у фикстур разбора: сцены собираются вокруг него.
    const CENTER: Vec2 = Vec2::new(MAP_SIZE.x / 2.0, MAP_SIZE.y / 2.0);

    /// Храм с заданной верой и формой.
    fn church(ring: Vec<Vec2>, faith: Faith, form: SacredForm) -> PolyArea {
        PolyArea {
            building_use: BuildingUse::Church(Sacred {
                faith,
                form,
                complex: 0,
                floor_dm: 0,
            }),
            ..building(ring, Vec::new())
        }
    }

    /// Сборка храмов — в одиночку, на трёх контурах: барабан внутри собора
    /// берёт его веру и его посев, а одинокая церковь без разметки — веру
    /// большинства города, и она же одна и попадает в счётчик угаданных.
    /// Тест строит [`ResolveFaithsCtx`] буквально, без карты.
    #[test]
    fn resolving_the_faiths_runs_on_its_own() {
        let mut buildings = vec![
            church(square(CENTER, 20.0), Faith::Orthodox, SacredForm::Nave),
            // барабан стоит в контуре собора и своей веры не имеет
            church(square(CENTER, 5.0), Faith::Unknown, SacredForm::Dome),
            // а эта церковь стоит сама по себе, и веры у неё тоже нет
            church(
                square(CENTER + Vec2::new(500.0, 0.0), 15.0),
                Faith::Unknown,
                SacredForm::Nave,
            ),
        ];

        assert_eq!(
            resolve_faiths(ResolveFaithsCtx {
                buildings: &mut buildings
            }),
            1,
            "по большинству города угадан ровно один храм"
        );

        let sacred = |building: &PolyArea| match building.building_use {
            BuildingUse::Church(sacred) => sacred,
            other => panic!("не храм: {other:?}"),
        };
        let (cathedral, drum, lone) = (
            sacred(&buildings[0]),
            sacred(&buildings[1]),
            sacred(&buildings[2]),
        );
        assert_ne!(cathedral.complex, 0, "посев храма не проставлен");
        assert_eq!(drum.faith, Faith::Orthodox, "барабан не взял веру собора");
        assert_eq!(drum.complex, cathedral.complex, "и красится сам по себе");
        assert_eq!(
            lone.faith,
            Faith::Orthodox,
            "одинокая церковь не взяла веру большинства"
        );
        assert_ne!(
            lone.complex, cathedral.complex,
            "чужой храм красится посевом собора"
        );
    }

    /// Контур без назначения, лежащий на храме, становится его пристройкой:
    /// вера — храма, форма — [`SacredForm::Annex`]. Контекст из одних домов —
    /// сцена из двух контуров, карты не нужно.
    #[test]
    fn a_house_lying_on_a_church_becomes_its_annex() {
        let mut buildings = vec![
            church(square(CENTER, 20.0), Faith::Orthodox, SacredForm::Nave),
            building(square(CENTER, 5.0), Vec::new()),
        ];
        resolve_faiths(ResolveFaithsCtx {
            buildings: &mut buildings,
        });
        assert_eq!(
            buildings[1].building_use,
            BuildingUse::Church(Sacred {
                faith: Faith::Orthodox,
                form: SacredForm::Annex,
                complex: sacred_of(&buildings[0]).complex,
                floor_dm: 0,
            }),
            "пристройка не взяла веру и посев храма"
        );
    }

    /// Храм контура целиком — сверить посев пристройки с посевом хозяина.
    fn sacred_of(building: &PolyArea) -> Sacred {
        match building.building_use {
            BuildingUse::Church(sacred) => sacred,
            other => panic!("не храм: {other:?}"),
        }
    }
}
