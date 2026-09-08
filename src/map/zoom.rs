//! Ступень зум-LOD слоя карты. Слой, у которого с зумом меняется сам рисунок
//! (путь — `map/rail.rs`, трамвай — `map/tram.rs`), держит свою таблицу
//! ступеней; ресурс-индекс в неё, выбор ступени по зуму и слежение за камерой
//! у таких слоёв общие — таблицы разные, механизм один.

use std::fmt;
use std::marker::PhantomData;

use bevy::camera_controller::pan_camera::PanCamera;
use bevy::prelude::*;

/// Таблица ступеней зум-LOD — тип-маркер слоя (пустой enum), по которому
/// [`ZoomBucket`] знает свои пороги. Зум — мировых метров на логический
/// пиксель (`PanCamera`).
pub trait ZoomLods: Send + Sync + 'static {
    /// Верхние (исключающие) границы ступеней в порядке таблицы; последняя —
    /// `f32::INFINITY`, чтобы любой зум попал в какую-то ступень.
    fn max_zooms() -> impl Iterator<Item = f32>;
}

/// Текущая ступень таблицы `T` — индекс. На входе в мир её ставит
/// [`seed_zoom_bucket`] по уже поставленной камере, дальше она меняется только
/// при пересечении порога зума ([`update_zoom_bucket`]), на что пересборка
/// слоя отвечает новым мешем (`retuned` в `map/mod.rs`). Не сохраняется —
/// её всегда диктует камера, а вид камеры хранится своей настройкой
/// (`SavedCameraView`).
#[derive(Resource)]
pub struct ZoomBucket<T: ZoomLods> {
    pub index: usize,
    _table: PhantomData<fn() -> T>,
}

impl<T: ZoomLods> ZoomBucket<T> {
    /// Первая ступень, чья граница выше зума. Зум на самой границе попадает в
    /// верхнюю ступень.
    pub fn for_zoom(zoom: f32) -> Self {
        let mut index = 0;
        for (candidate, max_zoom) in T::max_zooms().enumerate() {
            index = candidate;
            if zoom < max_zoom {
                break;
            }
        }
        Self {
            index,
            _table: PhantomData,
        }
    }
}

/// Значение до первого входа в мир не наблюдается — [`seed_zoom_bucket`] идёт
/// перед первой сборкой слоя. Дальняя ступень, а не ближняя: если порядок
/// когда-нибудь сломается, ошибка обойдётся самой дешёвой сборкой, а не самой
/// дорогой.
impl<T: ZoomLods> Default for ZoomBucket<T> {
    fn default() -> Self {
        Self::for_zoom(f32::INFINITY)
    }
}

// вручную, а не derive: derive навесил бы те же трейты на маркер `T`, которому
// они ни к чему
impl<T: ZoomLods> Clone for ZoomBucket<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ZoomLods> Copy for ZoomBucket<T> {}

impl<T: ZoomLods> PartialEq for ZoomBucket<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T: ZoomLods> Eq for ZoomBucket<T> {}

impl<T: ZoomLods> fmt::Debug for ZoomBucket<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ZoomBucket").field(&self.index).finish()
    }
}

/// Ступень зума по фактическому масштабу камеры. `set_if_neq` — чтобы
/// `resource_changed` срабатывал только на пересечении порога, а не каждый
/// кадр.
pub fn update_zoom_bucket<T: ZoomLods>(
    camera: Single<&PanCamera, With<Camera2d>>,
    mut bucket: ResMut<ZoomBucket<T>>,
) {
    bucket.set_if_neq(ZoomBucket::for_zoom(camera.zoom_factor));
}

/// Ступень на входе в мир — по камере, уже поставленной на стартовый вид
/// (`camera::place_camera_on_world_ready`): в режиме `save` мир открывается на
/// сохранённом зуме, при смене города — на `START_ZOOM`, и ни то, ни другое
/// не обязано совпадать со ступенью, оставшейся от прошлого мира. Без этого
/// первая сборка слоя шла по старой ступени, а первый же `Update` собирал его
/// заново — у пути это до 23 мс и 673 k вершин, выброшенных на входе.
///
/// Мимо детекции изменений: сборка слоя идёт следом в той же цепочке, и
/// пометка «изменился» заставила бы `retuned` собрать его второй раз в
/// первом `Update`.
pub fn seed_zoom_bucket<T: ZoomLods>(
    camera: Single<&PanCamera, With<Camera2d>>,
    mut bucket: ResMut<ZoomBucket<T>>,
) {
    *bucket.bypass_change_detection() = ZoomBucket::for_zoom(camera.zoom_factor);
}

#[cfg(test)]
mod tests;
