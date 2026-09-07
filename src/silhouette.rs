//! Силуэты пешек: процедурный атлас из трёх ячеек — диск человека, «уголёк»
//! демона и ореол, — посчитанный при старте по расстоянию до контура, плюс
//! пол размера в пикселях, ниже которого пешка на экране не ужимается.
//!
//! В `assets/` ни одного файла: художника у проекта нет, а форма, которую
//! видно с зума толпы, задаётся тремя формулами. Все три глифа лежат в **одном**
//! изображении, потому что спрайты батчатся по текстуре: люди и демоны
//! перемешаны по z (y-сортировка), и две текстуры резали бы батч на каждом
//! демоне. Прозрачные тексели несут цвет кромки, а не чёрный — иначе линейная
//! фильтрация подмешивала бы к краю чёрную рамку.
//!
//! Мипы считаются здесь же (усреднение 2×2): точка в 2–4 px с 64-пиксельной
//! текстуры без них искрит при каждом шаге пешки.

use std::f32::consts::FRAC_PI_2;

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Сторона ячейки атласа на нулевом мипе, px.
const CELL_PX: u32 = 64;
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
    /// Диск с тёмной каймой: человек, труп (растянутый в эллипс).
    Disc = 0,
    /// Семизубый уголёк с ярким ядром: демон.
    Ember = 1,
    /// Радиальное затухание: свечение вокруг демона и всякое «сияние».
    Halo = 2,
}

impl Glyph {
    const ALL: [Self; 3] = [Self::Disc, Self::Ember, Self::Halo];
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
    let image = images.add(atlas_image());
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

/// Нулевой мип: все ячейки подряд, RGBA8, строка 0 — верх.
fn rasterize_atlas(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for row in 0..height {
        for column in 0..width {
            let glyph = Glyph::ALL[(column / CELL_PX) as usize];
            let cell = Vec2::new(
                (column % CELL_PX) as f32 + 0.5,
                (height - 1 - row) as f32 + 0.5,
            );
            let p = cell / CELL_PX as f32 * 2.0 - 1.0;
            let (shade, alpha) = texel(glyph, p);
            let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
            data.extend_from_slice(&[byte(shade), byte(shade), byte(shade), byte(alpha)]);
        }
    }
    data
}

/// Тексель глифа в точке `p` ячейки (−1…1 по обеим осям): яркость (множитель к
/// цвету спрайта) и альфа.
fn texel(glyph: Glyph, p: Vec2) -> (f32, f32) {
    let edge = EDGE_PX * 2.0 / CELL_PX as f32;
    let r = p.length();
    match glyph {
        Glyph::Disc => {
            let inside = DISC_RADIUS - r;
            let alpha = smoothstep(-edge, edge, inside);
            let shade = RIM_SHADE
                + (1.0 - RIM_SHADE) * smoothstep(RIM_WIDTH - edge, RIM_WIDTH + edge, inside);
            (shade, alpha)
        }
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
    }
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
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
    fn every_glyph_is_transparent_outside_and_opaque_at_its_centre() {
        for glyph in Glyph::ALL {
            let (_, outside) = texel(glyph, Vec2::new(0.99, 0.99));
            assert_eq!(outside, 0.0, "{glyph:?} leaks past the cell corner");
            let (shade, centre) = texel(glyph, Vec2::ZERO);
            assert_eq!(centre, 1.0, "{glyph:?} is not solid at the centre");
            assert_eq!(shade, 1.0, "{glyph:?} is not brightest at the centre");
        }
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
        // 192×64 → … → 1×1: восемь уровней
        assert_eq!(image.texture_descriptor.mip_level_count, 8);
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
