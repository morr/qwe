//! Тротуары, которых нет в OSM: у жилой улицы без тега `sidewalk*` полосу
//! решает застройка вокруг ([`infer_sidewalks`]), а в сыром OSM достроенный
//! тротуар снимается ([`drop_untagged_sidewalks`]).

use crate::map::along::densify;
use crate::map::cars::district::Districts;
use crate::map::osm::model::{Highway, RoadLine, SidewalkSide};

/// Этажность квартала, с которой у жилой улицы без тега появляется тротуар.
/// Частный сектор читается как 1–2 этажа (`buildings::heights` даёт дому в
/// основном один этаж с мансардой), старая двух-трёхэтажка с палисадниками —
/// около двух с половиной; тротуар вдоль бордюра начинается там, где
/// застройка городская.
pub(super) const SIDEWALK_STOREYS_MIN: f32 = 3.0;
/// Шаг, с которым улица меряет застройку вдоль себя, м. Мера квартала
/// (`cars::district`) читает 120 м вокруг точки, так что пропустить квартал
/// при таком шаге нельзя.
const SIDEWALK_PROBE_STEP: f32 = 40.0;

/// Что решил [`infer_sidewalks`]: сколько улиц без тега было спрошено и у
/// скольких полоса снята.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct InferredSidewalks {
    pub(super) asked: usize,
    pub(super) dropped: usize,
}

impl std::fmt::Display for InferredSidewalks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { asked, dropped } = self;
        write!(
            f,
            "osm parse: {dropped} of {asked} untagged residential streets left without sidewalks"
        )
    }
}

/// Сырой OSM ([`super::RawOsm`]): тротуар без тега — наша достройка, и он снимается
/// со всех улиц; остаётся только тротуар по тегу `sidewalk*`. Возвращает, со
/// скольких сторон он снят.
pub(super) fn drop_untagged_sidewalks(roads: &mut [RoadLine]) -> usize {
    let mut dropped = 0;
    for side in roads.iter_mut().flat_map(|road| road.sidewalks.iter_mut()) {
        if *side == SidewalkSide::Inferred {
            *side = SidewalkSide::None;
            dropped += 1;
        }
    }
    dropped
}

/// Тротуар жилой улицы без тега `sidewalk*` — по застройке вокруг.
///
/// В OSM тротуар маппят исключениями: по Туле `sidewalk=no` у 44 way и
/// `separate` у 42, остальные молчат. В центре молчание значит «тротуар есть»,
/// в частном секторе — «его нет», и полоса в 1.2–3 м с каждой стороны улицы
/// между заборами была самой заметной неправдой колонки сравнения с
/// Яндексом (примеры 09, 13, боковые улицы 03 и 19). За ней шли зебры по
/// правилу: они ставятся только между тротуарами.
///
/// Решает средняя вдоль улицы **этажность квартала** — та же мера, которой
/// слой машин разреживает ряд у бордюра (`cars::district`): не меньше
/// [`SIDEWALK_STOREYS_MIN`] — тротуар с обеих сторон, как было; ниже или
/// вовсе без домов вокруг — обочина без полосы. Спрашиваются только жилая,
/// проезд без названия и жилая зона: у магистрали и улиц до `tertiary`
/// тротуар остаётся, у дворового проезда его не было и так, а грунтовой
/// улице полосу уже снял [`super::untagged_sidewalks`].
///
/// Кого спрашивать, говорит сама дорога: тротуар без тега —
/// [`SidewalkSide::Inferred`], и только такой здесь может быть снят. Мера
/// застройки — явный вход: [`Districts`] строится в [`super::finish_parse`] один раз
/// по всем домам (`storeys_at` — этажность квартала у точки, `None` — вокруг
/// пусто), и тест строит его прямо из домов сцены, без карты.
pub(super) fn infer_sidewalks(roads: &mut [RoadLine], districts: &Districts) -> InferredSidewalks {
    let asked: Vec<usize> = roads
        .iter()
        .enumerate()
        .filter(|(_, road)| {
            road.sidewalks.contains(&SidewalkSide::Inferred)
                && matches!(
                    road.highway,
                    Highway::Residential | Highway::Unclassified | Highway::LivingStreet
                )
        })
        .map(|(index, _)| index)
        .collect();
    let mut dropped = 0;
    for &index in &asked {
        let road = &mut roads[index];
        let storeys: Vec<f32> = densify(&road.points, SIDEWALK_PROBE_STEP)
            .into_iter()
            .filter_map(|point| districts.storeys_at(point))
            .collect();
        let mean = storeys.iter().sum::<f32>() / storeys.len().max(1) as f32;
        if storeys.is_empty() || mean < SIDEWALK_STOREYS_MIN {
            road.sidewalks = [SidewalkSide::None; 2];
            dropped += 1;
        }
    }
    InferredSidewalks {
        asked: asked.len(),
        dropped,
    }
}
