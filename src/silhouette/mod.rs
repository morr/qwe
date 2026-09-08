//! Силуэты пешек: процедурный атлас — диск человека, «уголёк» демона, ореол,
//! лужа крови и четыре позы лежащего тела ([`figure`]), — посчитанный при
//! старте по расстоянию до контура, плюс пол размера в пикселях, ниже
//! которого пешка на экране не ужимается.
//!
//! В `assets/` ни одного файла: художника у проекта нет, а форма, которую
//! видно с зума толпы, задаётся формулами. Все глифы лежат в **одном**
//! изображении, потому что спрайты батчатся по текстуре: люди, трупы и демоны
//! перемешаны по z, и две текстуры резали бы батч на каждом демоне.
//! Прозрачные тексели несут цвет кромки, а не чёрный — иначе линейная
//! фильтрация подмешивала бы к краю чёрную рамку.
//!
//! Мипы считаются здесь же (усреднение 2×2): точка в 2–4 px со 128-пиксельной
//! текстуры без них искрит при каждом шаге пешки.
//!
//! Посмотреть на атлас глазами:
//! `SILHOUETTE_DUMP=/tmp/atlas.png cargo test dump_atlas -- --ignored`.

use std::f32::consts::FRAC_PI_2;

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

pub mod figure;

/// Сторона ячейки атласа на нулевом мипе, px. 128, а не 64: ячейка трупа на
/// самом крупном зуме (0,05 м/px) — 75 логических px, на ретине 150
/// физических, и с 64 конечности фигуры расплывались бы.
const CELL_PX: u32 = 128;
/// Полуширина сглаживания кромки, px нулевого мипа.
const EDGE_PX: f32 = 1.0;
/// Радиус диска в долях полуячейки. Запас до края — на кромку и на глубокие
/// мипы, где сосед по атласу уже в двух текселях.
const DISC_RADIUS: f32 = 0.78;
/// Ширина тёмной каймы диска, в тех же долях: фишка с ободком, а не пятно.
const RIM_WIDTH: f32 = 0.16;
/// Яркость каймы — множитель к цвету спрайта: тот же цвет, темнее.
const RIM_SHADE: f32 = 0.45;
/// Уголёк: средний радиус, размах и число зубцов.
const EMBER_RADIUS: f32 = 0.60;
const EMBER_SPIKE: f32 = 0.20;
const EMBER_SPIKES: f32 = 7.0;
/// Яркость кончиков зубцов: ядро светится, края темнеют.
const EMBER_TIP_SHADE: f32 = 0.40;
/// Радиус ореола в долях полуячейки; за ним альфа ровно ноль.
const HALO_RADIUS: f32 = 0.98;

/// Ячейка атласа — форма, которую спрайт берёт по индексу.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    /// Диск с тёмной каймой: человек.
    Disc = 0,
    /// Семизубый уголёк с ярким ядром: демон.
    Ember = 1,
    /// Радиальное затухание: свечение вокруг демона и всякое «сияние».
    Halo = 2,
    /// Лужа крови под трупом: неровное пятно с брызгами.
    Pool = 3,
    /// Лежащие тела, четыре позы — [`figure`]. Идут подряд: [`Glyph::corpse`].
    Sprawled = 4,
    Prone = 5,
    Curled = 6,
    Crumpled = 7,
}

impl Glyph {
    const ALL: [Self; 8] = [
        Self::Disc,
        Self::Ember,
        Self::Halo,
        Self::Pool,
        Self::Sprawled,
        Self::Prone,
        Self::Curled,
        Self::Crumpled,
    ];
    const CORPSES: [Self; figure::POSES] =
        [Self::Sprawled, Self::Prone, Self::Curled, Self::Crumpled];

    /// Поза лежащего тела номер `pose` (по модулю числа поз).
    pub fn corpse(pose: usize) -> Self {
        Self::CORPSES[pose % figure::POSES]
    }

    /// Номер позы, если глиф — лежащее тело.
    fn pose(self) -> Option<usize> {
        Self::CORPSES.iter().position(|glyph| *glyph == self)
    }

    /// Куда под этот глиф ложится лужа, в координатах ячейки (−1…1): грудь
    /// фигуры; у остальных глифов — центр.
    pub fn pool_anchor(self) -> Vec2 {
        self.pose()
            .map_or(Vec2::ZERO, |pose| figure::Figure::pose(pose).pool_anchor())
    }
}

/// Переключает спрайт на другой глиф того же атласа; спрайт без атласа
/// (приложение без рендера) остаётся как был.
pub fn set_glyph(sprite: &mut Sprite, glyph: Glyph) {
    if let Some(atlas) = sprite.texture_atlas.as_mut() {
        atlas.index = glyph as usize;
    }
}

/// Атлас силуэтов. `None` — атлас не собран: приложение без рендера
/// (реплей, тесты) получает обычный квадратный `Sprite`, и это не ошибка.
#[derive(Resource, Default)]
pub struct Silhouettes(Option<Atlas>);

struct Atlas {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
}

impl Silhouettes {
    /// Спрайт с глифом `glyph`, покрашенный в `color`, размером `size` м.
    pub fn sprite(&self, glyph: Glyph, color: Color, size: Vec2) -> Sprite {
        let mut sprite = match &self.0 {
            Some(atlas) => Sprite::from_atlas_image(
                atlas.image.clone(),
                TextureAtlas {
                    layout: atlas.layout.clone(),
                    index: glyph as usize,
                },
            ),
            None => Sprite::default(),
        };
        sprite.color = color;
        sprite.custom_size = Some(size);
        sprite
    }
}

/// Как пешка стоит относительно камеры: тело в метрах и пол в логических
/// пикселях, ниже которого её спрайт не ужимается. На дальнем зуме метровый
/// человек — 0,2 px и исчезает, а толпа обязана читаться с любого расстояния;
/// пол превращает её в зерно, демона — в заметную точку.
///
/// `Sprite::custom_size` при этом принадлежит этому модулю: спавн задаёт тело,
/// [`size_fresh_silhouettes`] и [`resize_silhouettes_on_zoom`] пишут размер.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct Silhouette {
    /// Тело в метрах, как оно рисуется при 1 м = 1 px и крупнее.
    pub body: Vec2,
    /// Пол на экране, логических px по меньшей стороне тела.
    pub min_px: f32,
}

impl Silhouette {
    pub const fn new(body: Vec2, min_px: f32) -> Self {
        Self { body, min_px }
    }

    /// Размер спрайта при зуме `zoom` (метров в логическом пикселе): тело,
    /// растянутое до пола, если тело на экране меньше пола.
    pub fn drawn_size(&self, zoom: f32) -> Vec2 {
        let floor = zoom * self.min_px;
        let scale = (floor / self.body.min_element()).max(1.0);
        self.body * scale
    }
}

pub struct SilhouettePlugin;

impl Plugin for SilhouettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Silhouettes>()
            .register_type::<Silhouette>()
            .add_systems(Startup, build_atlas)
            .add_systems(
                Update,
                (size_fresh_silhouettes, resize_silhouettes_on_zoom).chain(),
            );
    }
}

/// Собирает атлас — один раз на процесс: глифы не зависят ни от города, ни от
/// прогона, а `Handle`-ы переживают любую смену мира.
fn build_atlas(
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut silhouettes: ResMut<Silhouettes>,
) {
    let started = std::time::Instant::now();
    let atlas = atlas_image();
    info!(
        "silhouette atlas: {}×{} px, {} mips, {} glyphs in {:.1?}",
        atlas.width(),
        atlas.height(),
        atlas.texture_descriptor.mip_level_count,
        Glyph::ALL.len(),
        started.elapsed()
    );
    let image = images.add(atlas);
    let layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(CELL_PX),
        Glyph::ALL.len() as u32,
        1,
        None,
        None,
    ));
    silhouettes.0 = Some(Atlas { image, layout });
}

/// Изображение атласа со всей цепочкой мипов.
fn atlas_image() -> Image {
    let width = CELL_PX * Glyph::ALL.len() as u32;
    let height = CELL_PX;
    let level0 = rasterize_atlas(width, height);

    let mut data = level0.clone();
    let mut levels = 1;
    let (mut w, mut h, mut level) = (width, height, level0.clone());
    while w > 1 || h > 1 {
        let (w2, h2, next) = downsample(w, h, &level);
        data.extend_from_slice(&next);
        levels += 1;
        (w, h, level) = (w2, h2, next);
    }

    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    // приложение целиком на `ImagePlugin::default_nearest()`, но силуэт —
    // гладкая фигура, а не пиксель-арт: кромка сглажена и должна остаться
    // такой на любом зуме
    image.sampler = ImageSampler::linear();
    image
}

/// Нулевой мип: все ячейки подряд, RGBA8, строка 0 — верх. Обход по ячейкам,
/// а не по строкам: фигура трупа собирается один раз на ячейку, не на тексель.
fn rasterize_atlas(width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 4) as usize];
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    for (cell, glyph) in Glyph::ALL.iter().enumerate() {
        let figure = glyph.pose().map(figure::Figure::pose);
        for row in 0..CELL_PX {
            for column in 0..CELL_PX {
                let centre = Vec2::new(column as f32 + 0.5, (CELL_PX - 1 - row) as f32 + 0.5);
                let p = centre / CELL_PX as f32 * 2.0 - 1.0;
                let (shade, alpha) = match &figure {
                    Some(figure) => figure.texel(p, edge()),
                    None => texel(*glyph, p),
                };
                let x = cell as u32 * CELL_PX + column;
                let i = ((row * width + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&[
                    byte(shade),
                    byte(shade),
                    byte(shade),
                    byte(alpha),
                ]);
            }
        }
    }
    data
}

/// Полуширина сглаживания кромки в единицах ячейки (−1…1).
fn edge() -> f32 {
    EDGE_PX * 2.0 / CELL_PX as f32
}

/// Тексель глифа в точке `p` ячейки (−1…1 по обеим осям): яркость (множитель к
/// цвету спрайта) и альфа. Для тел собирает фигуру на каждый вызов — путь для
/// тестов; атлас идёт через [`rasterize_atlas`].
fn texel(glyph: Glyph, p: Vec2) -> (f32, f32) {
    let edge = edge();
    if let Some(pose) = glyph.pose() {
        return figure::Figure::pose(pose).texel(p, edge);
    }
    let r = p.length();
    match glyph {
        Glyph::Disc => rimmed(DISC_RADIUS - r, edge, RIM_WIDTH, RIM_SHADE),
        Glyph::Ember => {
            // зубец смотрит вверх: фаза сдвинута на четверть оборота
            let theta = p.y.atan2(p.x) - FRAC_PI_2;
            let radius = EMBER_RADIUS + EMBER_SPIKE * (EMBER_SPIKES * theta).cos();
            let inside = radius - r;
            let alpha = smoothstep(-edge, edge, inside);
            let shade = 1.0 - (1.0 - EMBER_TIP_SHADE) * smoothstep(0.0, radius, r);
            (shade, alpha)
        }
        Glyph::Halo => {
            let falloff = (1.0 - r / HALO_RADIUS).clamp(0.0, 1.0);
            (1.0, falloff * falloff)
        }
        Glyph::Pool => figure::pool_texel(p, edge),
        Glyph::Sprawled | Glyph::Prone | Glyph::Curled | Glyph::Crumpled => {
            unreachable!("тела отданы фигуре выше")
        }
    }
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Тело с тёмной каймой — форма диска и форма фигуры: альфа по контуру,
/// яркость `rim_shade` у самого края и полная глубже каймы шириной `rim`.
/// `inside` — расстояние внутрь от контура; `inside`, `rim` и `edge` — в одних
/// единицах, каких именно, решает вызывающий (диск считает в долях полуячейки,
/// фигура — в ростах).
fn rimmed(inside: f32, edge: f32, rim: f32, rim_shade: f32) -> (f32, f32) {
    let alpha = smoothstep(-edge, edge, inside);
    let shade = rim_shade + (1.0 - rim_shade) * smoothstep(rim - edge, rim + edge, inside);
    (shade, alpha)
}

/// Следующий мип: среднее по блоку 2×2 (2×1 / 1×2 у вырожденной стороны).
fn downsample(width: u32, height: u32, data: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w2, h2) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity((w2 * h2 * 4) as usize);
    for y in 0..h2 {
        for x in 0..w2 {
            let mut sum = [0u32; 4];
            let mut taps = 0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = (x * 2 + dx).min(width - 1);
                    let sy = (y * 2 + dy).min(height - 1);
                    let i = ((sy * width + sx) * 4) as usize;
                    for (channel, acc) in sum.iter_mut().enumerate() {
                        *acc += u32::from(data[i + channel]);
                    }
                    taps += 1;
                }
            }
            out.extend(sum.iter().map(|acc| (acc / taps) as u8));
        }
    }
    (w2, h2, out)
}

/// Зум камеры — метров в логическом пикселе; `None` без камеры (реплей).
fn camera_zoom(camera: Option<Single<&Transform, With<Camera2d>>>) -> Option<f32> {
    camera.map(|camera| camera.scale.x)
}

/// Свежий или переписанный силуэт (спавн, труп) получает размер под текущий
/// зум — иначе пешка, родившаяся на дальнем плане, ждала бы первого щелчка
/// колеса.
pub fn size_fresh_silhouettes(
    camera: Option<Single<&Transform, With<Camera2d>>>,
    mut fresh: Query<(&Silhouette, &mut Sprite), Changed<Silhouette>>,
) {
    let Some(zoom) = camera_zoom(camera) else {
        return;
    };
    for (silhouette, mut sprite) in &mut fresh {
        sprite.custom_size = Some(silhouette.drawn_size(zoom));
    }
}

/// Полный проход по всем силуэтам — только на смену зума: тысячи записей в
/// `Sprite` за щелчок колеса, а не за кадр.
pub fn resize_silhouettes_on_zoom(
    camera: Option<Single<&Transform, With<Camera2d>>>,
    mut last_zoom: Local<f32>,
    mut all: Query<(&Silhouette, &mut Sprite)>,
) {
    let Some(zoom) = camera_zoom(camera) else {
        return;
    };
    if zoom == *last_zoom {
        return;
    }
    *last_zoom = zoom;
    for (silhouette, mut sprite) in &mut all {
        sprite.custom_size = Some(silhouette.drawn_size(zoom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_drawn_as_is_while_it_is_bigger_than_the_floor() {
        let silhouette = Silhouette::new(Vec2::splat(1.0), 2.0);
        // 0,4 м/px: метр — это 2,5 px, пол в 2 px не достигнут
        assert_eq!(silhouette.drawn_size(0.4), Vec2::splat(1.0));
    }

    #[test]
    fn body_is_stretched_to_the_floor_when_zoomed_out() {
        let silhouette = Silhouette::new(Vec2::new(1.6, 0.8), 2.0);
        // 4,5 м/px: пол — 9 м по меньшей стороне, пропорции сохранены
        assert_eq!(silhouette.drawn_size(4.5), Vec2::new(18.0, 9.0));
    }

    #[test]
    fn every_glyph_is_transparent_outside_and_opaque_at_its_anchor() {
        for glyph in Glyph::ALL {
            let (_, outside) = texel(glyph, Vec2::new(0.99, 0.99));
            assert_eq!(outside, 0.0, "{glyph:?} leaks past the cell corner");
            let (_, anchor) = texel(glyph, glyph.pool_anchor());
            assert_eq!(anchor, 1.0, "{glyph:?} is not solid at its anchor");
        }
        for glyph in [Glyph::Disc, Glyph::Ember, Glyph::Halo] {
            let (shade, _) = texel(glyph, Vec2::ZERO);
            assert_eq!(shade, 1.0, "{glyph:?} is not brightest at the centre");
        }
    }

    #[test]
    fn corpse_glyphs_are_the_four_poses_in_atlas_order() {
        assert_eq!(Glyph::corpse(0), Glyph::Sprawled);
        assert_eq!(Glyph::corpse(3), Glyph::Crumpled);
        assert_eq!(Glyph::corpse(4), Glyph::Sprawled);
        for (pose, glyph) in Glyph::CORPSES.into_iter().enumerate() {
            assert_eq!(glyph as usize, Glyph::Pool as usize + 1 + pose);
            assert_eq!(glyph.pose(), Some(pose));
        }
        assert_eq!(Glyph::Disc.pose(), None);
        assert_eq!(Glyph::Disc.pool_anchor(), Vec2::ZERO);
    }

    #[test]
    fn set_glyph_moves_the_atlas_index_and_leaves_a_plain_sprite_alone() {
        let mut plain = Sprite::default();
        set_glyph(&mut plain, Glyph::Prone);
        assert!(plain.texture_atlas.is_none());

        let mut atlas = Sprite::from_atlas_image(
            Handle::default(),
            TextureAtlas {
                layout: Handle::default(),
                index: Glyph::Disc as usize,
            },
        );
        set_glyph(&mut atlas, Glyph::Prone);
        assert_eq!(atlas.texture_atlas.unwrap().index, Glyph::Prone as usize);
    }

    /// Не проверка, а инструмент: пишет нулевой мип атласа в PNG, чтобы
    /// посмотреть на глифы глазами. `SILHOUETTE_DUMP=<путь> cargo test
    /// dump_atlas -- --ignored`; прозрачное лучше смотреть на сером фоне
    /// (`magick atlas.png -background gray50 -flatten out.png`).
    #[test]
    #[ignore = "пишет файл; запускать руками с SILHOUETTE_DUMP=<путь>"]
    fn dump_atlas() {
        let path = std::env::var("SILHOUETTE_DUMP").expect("SILHOUETTE_DUMP=<путь к png>");
        // кодировщику нужен ровно нулевой мип: цепочку отрезаем
        let mut image = atlas_image();
        let level0 = (image.width() * image.height() * 4) as usize;
        image.data.as_mut().expect("данные атласа").truncate(level0);
        image.texture_descriptor.mip_level_count = 1;
        image
            .try_into_dynamic()
            .expect("атлас — RGBA8")
            .save(&path)
            .expect("png не записался");
        println!("atlas written to {path}");
    }

    #[test]
    fn disc_rim_is_darker_than_its_core() {
        let (rim, alpha) = texel(Glyph::Disc, Vec2::new(DISC_RADIUS - RIM_WIDTH / 2.0, 0.0));
        assert_eq!(alpha, 1.0);
        assert!(rim < 0.5, "rim shade {rim} should be near RIM_SHADE");
    }

    #[test]
    fn mip_chain_ends_in_one_texel_and_keeps_the_layout() {
        let image = atlas_image();
        let width = CELL_PX * Glyph::ALL.len() as u32;
        // 1024×128 → … → 1×1: одиннадцать уровней
        assert_eq!(image.texture_descriptor.mip_level_count, 11);
        let mut expected = 0;
        let (mut w, mut h) = (width, CELL_PX);
        loop {
            expected += w * h * 4;
            if w == 1 && h == 1 {
                break;
            }
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
        }
        assert_eq!(image.data.as_ref().map(Vec::len), Some(expected as usize));
    }

    #[test]
    fn downsample_averages_the_block() {
        // 2×2 → 1×1: среднее четырёх текселей
        let data = [
            0, 0, 0, 0, 100, 100, 100, 100, 200, 200, 200, 200, 100, 100, 100, 100,
        ];
        let (w, h, out) = downsample(2, 2, &data);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![100, 100, 100, 100]);
    }
}
