//! Константы кровель — как они записаны в исходниках, а не их копия.
//!
//! Числа, которыми кровля задана, живут в двух файлах. **Фактура** — в
//! шейдере (`roof.wgsl`): шаг сетки размещения заплат, размер латки, доля
//! клеток у новой и у старой кровли. **Форма** — в `roofs.rs`: порог
//! заполнения описанного прямоугольника, доля вальмовых домов, вылет ската
//! вальмы и его зажим, уклон. Витрина печатает их рядом с картинкой — иначе
//! «на этой кровле заплат больше» и «эта вальма скорее фаска» не с чем
//! сопоставить, и подобрать константу по виду нельзя, не переключаясь на
//! редактор.
//!
//! Берутся они **из самих файлов** (`include_str!`), а не повторяются числом
//! в Rust: копия разошлась бы с оригиналом в первую же правку, и витрина
//! начала бы врать ровно про то, ради чего её открыли. `include_str!` — а не
//! чтение файла в рантайме — потому что и исходники, и витрина всё равно
//! пересобираются вместе, зато нет ни пути от текущего каталога, ни ожидания
//! загрузки ассета.
//!
//! Разбор нарочно наивный, строка вида `const ИМЯ: f32 = ЧИСЛО;` — и она же с
//! `u32` ради `HIPPED_SHARE`, доли вальмовых из десяти. Коды материалов
//! шейдера отсеиваются по типу — они `u32`, зеркало `RoofKind::code`, а не
//! настройка; `TAU` приходится исключать по имени.

/// Текст шейдера кровель, вшитый в витрину при сборке.
const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/roof.wgsl");

/// Исходник форм крыш — оттуда же и тем же способом.
const ROOFS_SOURCE: &str = include_str!("../../../src/map/buildings/roofs.rs");

/// Математическая постоянная, а не настройка фактуры.
const NOT_A_SETTING: [&str; 1] = ["TAU"];

/// Имя и значение каждой вещественной константы шейдера, в порядке файла.
/// Значение — тем же текстом, что и в коде: `0.28`, а не `0.280000`.
pub(crate) fn shader_constants() -> Vec<(&'static str, &'static str)> {
    SHADER_SOURCE
        .lines()
        .filter_map(|line| parse_const(line, ": f32 = "))
        .filter(|(name, _)| !NOT_A_SETTING.contains(name))
        .collect()
}

/// Константы выбора и построения формы крыши, в порядке `roofs.rs`.
/// Целочисленная среди них одна — `HIPPED_SHARE`, доля вальмовых из десяти, —
/// и без неё таблица не отвечала бы на «почему тут вальма».
pub(crate) fn shape_constants() -> Vec<(&'static str, &'static str)> {
    ROOFS_SOURCE
        .lines()
        .filter_map(|line| parse_const(line, ": f32 = ").or_else(|| parse_const(line, ": u32 = ")))
        .collect()
}

fn parse_const<'a>(line: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    let (name, value) = line.trim().strip_prefix("const ")?.split_once(kind)?;
    Some((name, value.strip_suffix(';')?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Разбор держится на форме строки в чужом файле, поэтому пустой список —
    /// не «констант нет», а «формат разъехался»: без этой проверки витрина
    /// молча покажет пустую группу.
    ///
    /// Тесты примеров `cargo test` не запускает (он их только собирает), так
    /// что этот идёт под `cargo test --examples`.
    #[test]
    fn the_patch_constants_are_read_from_the_shader() {
        let constants = shader_constants();
        assert!(
            constants.iter().any(|(name, _)| *name == "PATCH_SHARE_OLD"),
            "в `roof.wgsl` не нашлось ни одной вещественной константы: {constants:?}"
        );
        assert!(
            !constants.iter().any(|(name, _)| *name == "TAU"),
            "TAU — не настройка фактуры: {constants:?}"
        );
    }

    /// То же и по той же причине для `roofs.rs`: пустая группа выглядела бы
    /// как «констант формы нет», а не как «разбор разъехался».
    #[test]
    fn the_shape_constants_are_read_from_the_source() {
        let constants = shape_constants();
        for expected in ["RECT_FILL_MIN", "HIP_INSET", "HIPPED_SHARE"] {
            assert!(
                constants.iter().any(|(name, _)| *name == expected),
                "в `roofs.rs` не нашлось {expected}: {constants:?}"
            );
        }
    }
}
