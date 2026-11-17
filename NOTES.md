# NOTES — deep06 (план углубления, пункт №06: RequestLifecycle, шаги 1–2)

Задача: docs/deepening-plan.md в main checkout (незакоммичен — не коммитить) § 06,
только шаги 1–2; шаг 3 (тип-владелец фаз Queued/InFlight/Answered/Retired) отложен,
не открывать. Записи прошлых сессий (deep07, deep02, deep03, deep01, split-giants) — ниже.

## Решения

1. **Модули — `movement/pathfinding/budget.rs` и `movement/pathfinding/accept.rs`,
   каждый с `tests.rs` рядом** (паттерн split: родитель `pathfinding.rs` держит
   диспетчеры и приёмники,subjects живут в каталоге). Прецедент — `movement/step/`
   с его `tests.rs`: тесты по значениям, без `App`.
2. **`QueuedRequest` — `pub`, поля `pub(super)`.** Тип сидит в сигнатуре
   `dispatch_pathfinding_requests_deterministic`, ре-экспортируемой наверх
   (`movement/mod.rs`, используется `determinism` и `movement/tests.rs`):
   `pub(crate)` дал hard error `private_interfaces` в `cargo build` (на master
   тип был `pub` в приватном модуле — та же видимость, что у `pub` в приватном
   `budget.rs`). Поля строит только диспетчер в pathfinding.rs.
3. **Тесты очереди (10)** — формула единиц от константы (`1 + Чебышёв/
   PATHFINDING_UNIT_TILES`, ступени ровно на границе делителя), гистерезис
   подрезки ×2 (до двойного буфер растёт свободно), подрезка хранит самые
   ранние ключи, «не влезла — обрывает, а не пропускается» (дорогое в голове
   блокирует всё, уезжает ноль), точный влёт в бюджет (сумма == budget берётся
   целиком), FIFO-ключ через поведение сортировки (тик → вид → номер),
   `debug_assert` уникальности ключа пинится `#[should_panic]`.
4. **Тесты приёмки (9)** — устаревший ответ ничего не трогает; ответ пешке,
   которая уже не ждёт (состояние ушло), — тоже; спасение с непроходимого
   (телепорт, оба конца интерполяции, Idle, новое место проходимо); спасённый
   человек попадает в человеческую сетку (`any_in_cells_around`); недостижимая
   цель → `PathfindingError` без телепорта; приход из одной точки — путь снят
   ДО `to_idle` и событие доезжает (observer в сыром World ловит
   `MovableReachedDestinationEvent`); свежий путь начинается со второго
   waypoint; подрезка режет и равную дистанцию (граница `<=`); предел подрезки
   — ровно `REPATH_TRIM_LIMIT` срезов.
5. **Техника тестов приёмки** — системные параметры в сыром `World` через
   `SystemState<(Diagnostics, Commands, Option<ResMut<SpatialGrid<Human>>>)>`:
   компоненты вынимаются `entity_mut(e).take::<T>()` ДО `get_mut` (иначе займ
   мира пересекается), `params.apply(world)` применяет очередь команд и
   триггерит наблюдателя прихода. `DiagnosticsStore::default()` вставляется
   ресурсом — `Res` внутри `Diagnostics` обязателен. `bevy_ecs` 0.20: 
   `SystemState::get_mut` возвращает `Result`.
6. **Найдено по ходу**: `Movable` несёт `#[require(.. NeedsWanderTarget ..)]` —
   маркер есть у любой пешки с рождения, поэтому «маркера нет» в тестах
   устаревшего/ушедшего ответа не проверяется (не сигнал, а свойство спавна).
   `request_units` несёт мёртвый `.max(0)` (abs() уже неотрицателен) — оставлен
   как был: извлечение без смены поведения.
7. **Доки** — новых терминов модуль не вводит (глоссарийные *Dispatch rate* и
   *RequestedAt* не изменились), поэтому CONTEXT.md получил только указатель
   «где живёт» в *Dispatch rate*; determinism SKILL.md — указатели в «Dispatch
   rate» и «The FIFO key»; navigation-deep `references/pipeline.md` — шапка и
   упоминание `accept_answer` назвали оба файла.

## Статус

- [x] worktree .worktrees/deep06 (master b7e89340), target засеян CoW, assets/osm
      rsync -a (mtime сохранён)
- [x] скиллы git/bevy/navigation-deep/determinism загружены до первого гейта
- [x] 59d9c5bb шаг 1: бюджетная очередь → budget.rs + 10 юнит-тестов
- [x] d0140369 шаг 2: accept_answer → accept.rs + 9 юнит-тестов
- [x] e34bca09 фикс лайков серии: QueuedRequest обратно pub (private_interfaces),
      drop() только у счётчика (у системных параметров Drop нет) + rustfmt-перекладки
- [x] check.sh OK: 1583 lib-теста (master 1564 + 19 новых), clippy чист, fmt чист;
      tests/determinism 5/5 — реплеи зелёные (конвейер ответов тронут перемещением)

## Вердикт по перфу

Перемещение кода без новой работы: ни на кадр (диспетчеры и приёмники зовут те же
функции через `use self::`), ни на элемент загрузки (очередь и приёмка работают
только в `FixedUpdate`/`Update` и при ответах поиска — путей загрузки это не
касается). Строки и вызовы те же; `cargo test` полный зелёный. Замер живого
приложения не нужен.

---

# NOTES — deep07 (план углубления, пункт №07: Census — счётчики один раз)

Задача: docs/deepening-plan.md в main checkout (незакоммичен — не коммитить) § 07,
плюс влитая из обзора №2 часть (репортёр спайков, реестр диагностик, env-порог — Resource).
Записи прошлых сессий (deep02, deep03, deep01, split-giants) — ниже.

## Решения

1. **Модуль — `src/census.rs`, плагин рядом с `TelemetryPlugin`.** Census —
   производные счётчики (пересчитываются с нуля каждый кадр), Telemetry —
   накопительные исходы; сброс Census при рестарте не нужен, `WorldStarted` его не
   трогает. BRP-читаемый: `res get Census` заменяет связку `count Human` +
   `res get Telemetry`.
2. **Фаза проверки инварианта — `PlayPhase::Live`, не `Playing`.** На прогреве ПОСЛЕ
   рестарта телеметрия ещё держит счёт предыдущего прогона (сброс — `WorldStarted`,
   звенит только на входе в `Live`), а `alive` уже свежий — проверка в `Warmup`
   падала бы на каждом рестарте с убитыми. Пин:
   `the_invariant_is_checked_only_in_live`.
3. **Порядок с потребителем — ребро на стороне потребителя** (прецедент
   `camera.rs` → `before(sim_time::begin_sim_load)`):
   `sync_world_counts.after(crate::census::count_census)`. Модуль переписи не
   импортирует ui; обе системы живут только в игре — ребро висячим не бывает.
4. **Компоненты переписи регистрируются плагинами при сборке** (human/demon/movement
   держат `register_type` ради BRP) — простые запросы валидны с первого кадра,
   `Option<Query>` не нужны. 8 счётчиков: humans/demons/corpses/souls/fleeing/
   wandering/moving/chasing; demons в строку спайка не входит, но в переписи живёт
   (HUD читает).
5. **Fingerprint реплея не тронут.** Его `moving` — «не стоит на месте»
   (Pathfinding+Moving+Error по строкам отпечатка), а не тег
   `MovableStateMovingTag` переписи; семантики разные, а свободный пересчёт из
   собственных строк — его защита от «двух одинаково неподвижных миров».
   killed/escaped остаются в Telemetry (инкрементальные по событиям) — план это
   требует прямо.
6. **Репортёр спайков — отдельная система** после `end_sim_load` в
   `AfterFixedMainLoop`; сырая пара `frame_sim_ms`/`frame_ticks` хранится в
   `SimLoad` (потребители: `frame_overrun` и репортёр). Порог — ресурс
   `TickSpikeThreshold` (`QWE_PERF_FRAME_SPIKES` при сборке, BRP-правится живьём),
   **0 = выключен**.
7. **Найденный дрейф**: старый `tick_spike_threshold` (OnceLock, `unwrap_or(0.0)`)
   с условием `per_tick > 0.0` писал строку спайка **каждый кадр** без
   env-переменной, вопреки своему доку «без переменной лог выключен». Репортёр
   проверяет `threshold.0 <= 0.0 → выключено`.
8. **Кортеж плагинов main.rs делился надвое** — шестнадцатый плагин за лимитом
   `Plugins` для 15-элементных кортежей: мир (8) и косметика (8), комментарий у
   кортежа обновлён.
9. **Реестр `SPIKE_PARTS`** в diagnostics.rs — короткие имена строк лога + пути;
   регистрация десяти `sim/*_ms` циклом по нему же (порядок регистрации
   изменился — читателей порядка нет), порядок разреза в логе сохранён прежний.

## Замер перфа (Тула, 10x, QWE_PERF_NOAPPNAP=1 caffeinate -i, BRP_PORT=15709)

Фазово-согласованные пробы, мир осевший, по одной цепочке команд:

| | master (a8e75520) | deep07 |
|---|---|---|
| SimLoad.tick_ms | 0.255 / 0.259 / 0.254 (тик 23340) | 0.228 / 0.224 / 0.225 (тик 12587) |
| affordable / pipeline / actual | 30 / 30 / 10.0 | 30 / 30 / 10.0 |
| «tick spike» строк в логе без env | **31791** (~4 мин прогона) | **0** |

Тиковый путь изменением не тронут; разница в пользу ветки — мир моложе у ветки,
разброс оседания (deep01 мерил ±9 % между заездами одного кода). Собственная цена
переписи — 8 сумм размеров архетипов за кадр (`iter().len()` при чисто архетипном
фильтре — без прохода по сущностям) плюс debug-инвариант в dev-профиле: доли
микросекунды на кадр, от населения не зависит. Репортёр спайков в спокойном кадре —
один ранний return. **Вердикт: стоимость ничтожна, остаётся**; заодно закрыт
лог-шум master (3 строки спама в кадр).

## Живая проверка

- master без `QWE_PERF_FRAME_SPIKES` писал «tick spike» **каждый кадр** (31791 строк
  за прогон) — дрейф из п.7 подтверждён на живом приложении; deep07 — 0 строк.
- `res get Census` отвечает всеми восемью счётчиками (ресурс зарегистрирован).
- Инвариант на паузе: Human 19119 + killed 881 + escaped 0 == 20000;
  CorpseTag 881 == killed; HUD показывает 19119 / 881 — значения переписи доезжают
  до панели.
- Паник и ERROR в логе нет.

## Статус

- [x] worktree .worktrees/deep07 (master a8e75520), target засеян, assets/osm rsync -a
- [x] скиллы git/bevy/species-behavior/sim-speed/determinism (+ ui-panels, live-app)
      загружены до первого гейтованного действия
- [x] c1 add Census — e9de308b (census.rs + lib/main/stats + CONTEXT.md +
      species-behavior + ui-panels)
- [x] c2 extract tick spike reporter — e16dbbea (sim_time.rs + diagnostics.rs +
      sim-speed)
- [x] тесты: census 2/2, детерминизм 5/5 (Fingerprint не менялся), check.sh OK —
      1564 lib + интеграционные, clippy чист, fmt применён
- [x] замер перфа + живая проверка (см. выше)
- [x] NOTES закоммичены, ff master

---

# NOTES — deep02 (план углубления, пункт №02: BuildingClass — классификация один раз)

Задача: docs/deepening-plan.md в main checkout (незакоммичен — не коммитить) § 02,
два шага; шаг 3 (temples → Option<SacredPlan>) — не в этом заезде.
Записи прошлых сессий (deep03, deep01, split-giants) — ниже.

## Найденный дрейф (что сводим)

- **kind_of (кровля) vs wall_kind_of (стены)** на `BuildingUse::Other`: кровля — только
  пятно ≤ SMALL_FOOTPRINT_MAX (250) → HOUSE_ROOFS; стены — пятно **и** `storeys <
  LOW_RISE_STOREYS` (4) → HOUSE_WALLS. Тегированная мелкая коробка в 27 м: кровля —
  черепица + скатная когорта (is_pitched тоже без сторис-гварда), стены — панель
  (APARTMENTS_WALLS) и балконы. Разрешение в пользу правила стен (гвард этажности):
  частный дом по определению малоэтажный (HOUSE_HEIGHTS 3–6.4 м, «тег всегда сильнее
  вывода»), девятиэтажка с черепичной двускатной — артефакт дрейфа. is_pitched получает
  тот же гвард.
- Два матча Roofing (flat в `facade_and_roof_builders` и 2.5D в
  `push_house_with_arches`) семантически сходятся, но держатся вручную: дымоход
  (`clutter && ridge`), вальма всегда, шатёр никогда, merlons после. Унификация — один
  `push_roofing`, flat зовёт его с lift=ZERO и lean=None (вырожденный случай: конёк не
  поднимается, фронтонов и слуховых окон нет — они требуют подъёма).
- `Sanctuary::of` ×5 (facade 955, extrusion 1091, push_house 1246 — витринный, остаётся,
  sweeps 105, roof_shadows 276), `garage_runs` ×3 (facade 951, extrusion 1090, отчёт
  mod.rs 608) → `CityContext` в mesh_buildings.

## Дизайн

- **`classify.rs`**: `BuildingKind` (Fortress{tower}, Temple, Garage, House, Shed,
  Housing(Shape), DeepHull(Shape), Office(Shape), BigBox, Shop, Store, Hall, Public) —
  единственный матч по building_use × размеры × пороги; `BuildingClass { kind, height,
  storeys, pitched }`, `classify(building, fp)` считает высоту (тег или вывод вида) и
  этажность (высота / ярус вида — tiers переезжают из layers.rs; STOREY_HEIGHT остаётся в
  settings.rs). Гвард дом-кандидата: `Other` ≤ 250 м² — частный сектор только при
  `storeys < LOW_RISE_STOREYS` (= layers::BALCONY_STOREYS_MIN, константа остаётся в
  layers.rs ради wall_gallery-парсера), иначе Housing/Low. Для нетегированных гвард
  проходит всегда (HOUSE_HEIGHTS ≤ 2 этажей) — поведение вывода высоты не меняется.
- Таблицы остаются у владельцев: высотные — heights.rs (`inferred_height(kind, …)`),
  палитры стен/крыш — material.rs, оборудование — clutter.rs; на класс переходит
  **выбор ветки**. thresholds формы (SLAB_*, LOW_FOOTPRINT_MAX, TOWER_*, STORE_FOOTPRINT_MIN)
  и is_deep_hull переезжают в classify.rs. SMALL_FOOTPRINT_MAX/SHED_FOOTPRINT_MAX
  остаются в roofs.rs (владелец скатной когорты).
- Классify идёт **параллельным массивом** к footprints: `Vec<BuildingClass>` в
  mesh_buildings/measure_layers, билдеры получают срезы. Класс fp-независим по значению
  (storeys_by_shape и is_deep_hull дают то же без fp) — cars::height_or_default и
  fp=None-вызовы живут как жили.
- **`CityContext`** (mod.rs): { sanctuary, garage_runs, arches } — один расчёт на сборку;
  arches теперь для обоих режимов (flat их резал своим вызовом — строка `arches` бенча в
  плоских режимах станет ненулевой, строка facades+roofs полегчает на ту же величину).
- **HouseLook** — общий шаг сборки вида дома: `house_look(building, run, class, fp) ->
  HouseLook { roof, wall (с оглядкой на прогон), facade (без прогона — полоса) }`; оба
  режима зовут один шаг.
- `is_pitched` удаляется: `BuildingClass::is_pitched()` (kind House|Shed && без дворов).
  Тесты `a_pitched_roof_goes_on_houses_and_small_boxes_only`,
  `a_hull_deeper_than_housing_carries_no_balconies` переезжают на классификатор.

## Коммиты (план)

1. dcb126c1 classify.rs + heights на класс + таблица здание→класс (3 теста: таблица
   «здание → класс», этажность/ярус, скатная когорта)
2. 5bdea9f6 material (кровля+стены) на класс — разрешение дрейфа в этом коммите;
   is_deep_hull + shape-пороги переехали в classify.rs
3. 7bb65199 скатная когорта (is_pitched удалён → BuildingClass::is_pitched)
4. 840db379 clutter на класс (is_landmark → BuildingClass::is_landmark)
5. a0948ad3 storeys_of умирает, ярусы — только в classify.rs
6. ace970ad CityContext { sanctuary, garage_runs(=runs), arches } в mesh_buildings;
   arches теперь для обоих режимов; бенч: строка `arches` → `context`
7. a04e075d push_roofing — общий матч крыши, flat = вызов с lift=ZERO/lean=None;
   HouseLook = общий шаг вида дома (roof/wall/facade)
8. 6e45b958 docs: CONTEXT.md (Building class, City context) + osm-map (SKILL.md,
   buildings-heights/material/clutter/landmarks)
+ 8c702307 фикс компиляции серии (импорты, seed в kind_of, CityContext приватный,
  тест рампы тонировки — его высокая фикстура опиралась на дрейф: 60-м тегированная
  коробка брала черепицу частного сектора над панельными стенами; обе фикстуры теперь
  Apartments)
+ 535ca972 misc rustfmt

## Решения сверх плана

- **Класс не тредится в билдеры срезом** (моё расширение плана — отменено):BuildingClass
  считается тем, кому нужен (`classify(building, fp)` — хеш + матч; min_area_rect только
  там, где и раньше), CityContext — только три городовых входа по плану. Причина: срез
  классов ломал бы витринные двери (push_house/wall_of) и добавлял параллельный массив
  к четырём существующим без меры в плане.
- **roof_look/wall_look считают класс внутри себя** (промежуточно до шага 6): тесты
  (около 20 мест roof_look/wall_look) остаются на прежних сигнатурах; цена — повторный
  classify на потребителя (~нс, хеш + матч).
- **Тест roof_tint_darkens...** — единственный mesh-тест, тронутый по существу: его
  «высотка» против «базы» сравнивала разные материалы (частная черепица у тегированной
  60-м коробки — сам дрейф). Обе фикстуры заданы Apartments — сравнение материалов
  одинаковое.

## Статус

- [x] worktree .worktrees/deep02 + seed target (10 s CoW) + rsync assets/osm (mtime)
- [x] скиллы git/bevy/osm-map загружены; осмотр закончен
- [x] коммиты 1–8 + фиксы
- [x] check.sh из worktree: OK (build + все тесты зелёные + clippy чист + fmt)
- [x] бенч map_meshing (Тула, release) — чисто во всех режимах, вердикт выше
- [ ] ff master, spawn №07 (Census)

## Урок заезда

**cargo-проверки обязаны идти с cwd worktree.** Первые пять циклов «проверил — зелёный»
компилировали master: сессия стоит в main checkout, `cargo check` без cd проверял не тот
дерево, а его чистоту. Поймалось только на check.sh из абсолютного пути worktree
(скрипт делает cd от $0): 6 ошибок компиляции + 8 warning'ов, которые «зелёные» прогоны
пропустили. Для быстрых проверок: `cd <worktree> && cargo check --all-targets`; для
полной — абсолютный путь к `tools/check.sh` (он сам cd-ится).

## Вердикт по перфу

**На кадр новой работы нет** — весь пункт живёт в load-стороне. На элемент загрузки
арифметика однозначно положительная: CityContext сжал Sanctuary::of ×5 → ×1,
garage_runs ×3 → ×1, arches_by_building ×2 → ×1 — на Туле это ~4 лишних прохода по
14 598 зданий на сборку, теперь один (`context` 1–2 мс на весь расчёт в бенче).
classify добавляет на потребителя хеш первой вершины + матч (наносекунды);
min_area_rect считается там же, где считался раньше (fp передан). Живой прогон не
нужен — консолидация, а не новое поведение; бенч `map_meshing` на Туле прошёл чисто
во всех пяти режимах (extruded 19–35 мс, facades+roofs 39–56, shadows ~66, roof
shadows 5–7 — порядки журнальные). Строка бенча `facades+roofs` стала чуть легче
прошлой (`arches` ушла в `context`) — задокументировано в коммите ace970ad.

---

# NOTES — deep03 (план углубления, пункт №03: TickStage — порядок тика списком)

Задача: docs/deepening-plan.md в main checkout (незакоммичен — не коммитить) § 03.
Записи прошлых сессий (deep01, split-giants) — ниже.

## Решения

1. **Модуль — `src/tick_stage.rs`, поднимается из `SpatialPlugin`, не из `SimBootPlugin`.**
   План называл SimBootPlugin/sim_yard, но два адресата, которым порядок нужен, поднимают
   плагины без SimBootPlugin: `crowd_demo` и `tests/movement.rs` (SpatialPlugin + MovementPlugin).
   `SpatialPlugin` — место, где жила и цепочка `SimSet`; все сборки, кому нужен порядок тика,
   его поднимают. Ассембли без `SpatialPlugin` (лестницы поведения в тестах) цепочку не
   настраивают — им порядок между плагинами не нужен.
2. **Стадии — только порядок, без гейта.** У цепочки `SimSet` был `run_if(in_state(Playing))`;
   в мире `TickStage` гейта нет, иначе попавшие в стадии системы движения гейтнулись бы в
   сборках без `Playing` (crowd_demo до установки состояния). Гейт старой цепочки, который
   нёс `rebuild_demon_grid`, переехал на саму систему явным `run_if`. Гейт на мир у систем
   поведения и движения остался на `SimPipeline`, доктрина конвейера не тронута.
3. **Попарные рёбра вместо `.chain()`.** Конфиг сетов Bevy 0.20 берёт кортежи (до 20),
   но не `Vec` — кортеж рядом с перечнем стадий значил бы держать порядок дважды (та
   теневая копия, которую пункт убирает). Рёбра `next.after(prev)` в цикле по
   `TickStage::ORDER` дают тот же транзитивный порядок; auto-insert sync points работает
   на сплющенных зависимостях — ребро Spawner→DemonGrid даёт ту же точку синхронизации
   команд спавна, что было ребро спавнера к `SpatialRebuild` (пин
   `a_demon_born_on_a_tick_enters_the_demon_grid_on_that_same_tick` зелёный).
4. **rise_souls — в хвост человеческой цепочки** (4-й элемент кортежа после
   `panic/flee/escape`): ребро к сету и цепочка сводятся к одному набору зависимостей
   (после всех систем поведения, до стадии `Move`). Количество систем в
   `SimPipeline::BothModes` в пине `human_behavior_does_not_run_outside_the_world`
   не изменилось (4).
5. **Пробы — по пробе на стык перечня**: `head` до `Guard`, по пробе после каждой стадии
   (`TickStage::probe_name`). Имена сегментов в разрезе спайка сменились:
   `head/spatial/demon/human/move/tail` → `head` + `guard/tick/answers/spawner/demon_grid/
   demon_behavior/human_behavior/move/separation/dispatch`. Единственный потребитель —
   строка `tick spike segments` в логе; парсеров имён нет.
6. **advance_sim_tick остался регистрироваться movement-ом** (как и был) — перенос
   регистрации в `determinism` не входил в пункт, аддитивная миграция его не трогает.

## Замер перфа (Тула, 10x, caffeinate -i, BRP_PORT=15709, SimLoad.tick_ms)

Фазово-согласованные пробы (одинаковая фаза мира, ~50–95 сим-секунд, тики 3.3–8.7k):

| | master (7ef9ae24) | deep03 |
|---|---|---|
| tick_ms | 0.269 / 0.290 / 0.292 | 0.291 / 0.292 / 0.307 |

Разница ~0.01–0.02 мс/тик (~5%) — цена 11 проб-систем на тик (два `Instant::now` + push
каждая), на границе шума оседания мира. На кадр при 30x ≈ 0.3% стены. Регулятор не тронут
(affordable 13–25, pipeline 30, wait 0 в обоих прогонах). Полностью осевший мир ветки на
тиках ~85–90k — 0.244–0.248 мс. Вердикт: стоимость ничтожна, остаётся.

## Статус

- check.sh OK: 1559 passed + 0 failed, clippy чист, rustfmt применён.
- Живой прогон: мир поднимается на новом расписании (0 паник), pathfinding ходит,
  спавнер/погоня работают.
- ff master на голову deep03 — чистый (master стоял на 7ef9ae24, точке ветвления).
- Следующий заезд плана — №02 BuildingClass, через zcode-sessions `spawn_session`,
  `task: false`, модель Start plan (указание владельца).

---

# NOTES — deep01 (план углубления, пункт №01: решётка предикатов MapData)

Задача: docs/deepening-plan.md в main checkout (незакоммичен — не коммитить) § 01,
плюс влитая из обзора №2 часть (RawOsm-наборы, RailLine-решётка). Ниже — решения
этой сессии; запись прошлой сессии (split-giants) — под ней.

## Решения по расхождениям

1. **is_street() в rail/parking — замысел, не дрейф.** `is_at_grade()` = Street &&
   !bridge && !passage — про уровень земли, не про уличность: проезд тоже занимает
   землю (переезд кладётся на проезд — rail/tests.rs:571 «проезд» зелёный; кромка
   стоянки дотягивается до проезда). is_street() остаётся добавкой is_carriageway.
   Доказательство замысла: доки rail.rs::crossable («проезжая часть любого класса»),
   parking.rs::is_through (oneway||roundabout проезды считает), parking.rs фильтр
   streets (проезды легитимные дороги доступа).
2. **Мост в is_carriageway, но не в is_at_grade.** is_carriageway не выражается через
   is_at_grade && is_street — мост бы выпал. Решётка: carves_navmesh (bridge||passage,
   любой класс), is_at_grade (Street, на земле), is_carriageway (Street && !passage &&
   is_street, мост включён). Пересекаются, не вкладываются.
3. **pavement-аксессора нет** — случая «потребитель решает None» не существует:
   None-ответы уже в предикатах модели (is_paved_path=false → тропинка; задокументировано
   в поле). Только lanes переезжает в модель (RoadLine::lanes() от ширины).
4. **RailLine-решётка**: is_ground_track() = kind != Tram && !bridge (rail.rs
   level crossings, wagons::holds_stock + kind==Active); is_track_bridge() = kind !=
   Tram && bridge (roads.rs:1426). cars/rails.rs keepout остаётся `kind != Tram` —
   мост включён НАМЕРЕННО (док модуля: машина поверх плиты путепровода).
5. **MapData-запрос** `at_grade_roads()`: потребители parse/rails.rs::crosses_street и
   parking (ParkingLayout::new переезжает на &MapData — все 7 вызовов и так передавали
   (&map.parking, &map.roads, map.traffic_side)); roads.rs:1434 остаётся на drawn
   (подмены классов: переезд через тротуар рисуется улицей — спрашивать надо
   нарисованную дорогу, не модельную).
6. **RawOsm** (обзор №2): перечень достроек отрисовки — в коде (enum DrawBuildout +
   RawOsm::disables(what)), шесть веток потребителей называют свою достройку;
   parse-проходы остаются своим набором (skips_passes, перечисление — тело
   finish_parse). draws_raw() уходит вовсе.
7. **spawn.rs AreaKind** — без MapData-запроса (один потребитель): Landuse и Pocket
   расходятся по своим петлям-итераторам по видам (карманы несут только Grass|Residential,
   landuse только Residential|Industrial — parse.rs::push_area / pockets.rs:463-465).

## Статус

- [x] worktree + rsync assets/osm (mtime сохранён) + seed target
- [x] скиллы git/bevy/osm-map загружены; осмотр кода закончен
- [x] c1 is_at_grade + таблица тестов + доки (509c7afd)
- [x] c2 rail crossable сведение (d283ace6)
- [x] c3 parking сведение (d230aabe)
- [x] c4 RailLine решётка (ecdb5681)
- [x] c5 MapData::at_grade_roads + ParkingLayout::new(&MapData) (b90c3ec3 + тест cdd67d78)
- [x] c6 lanes accessor (6463afb8)
- [x] c7 DrawBuildout (8bba570b)
- [x] c8 spawn по видам (48717078)
- [x] check.sh OK (нужен был фикс needless borrow — 460824d4; fmt 4549a47a)
- [x] rebase на уехавший master (9 коммитов navigation/, конфликтов нет), check.sh после
      ребейза OK
- [x] замеры перфа (см. ниже)
- [ ] ff master — ЗАБЛОКИРОВАН: в main checkout идёт параллельная сессия с
      незакоммиченными правками (10 файлов, среди них CONTEXT.md и
      examples/demos/roads/main.rs — оба в моём диффе). Ветка deep01 (50c4e88e)
      уже ребейзнута на master (5b6871f9) и готова к `git merge --ff-only deep01`
      из main checkout, как только его дерево станет чистым.

## Замеры перфа (Тула, тёплый старт, caffeinate -i, QWE_PERF_NOAPPNAP=1)

Первый baseline-прогон master был загрязнён (полная пересборка 51 с перед замером —
все фазы равномерно ×1.7); чистый второй прогон против прогона ветки:

| строка лога | master | deep01, заезд 1 | deep01, заезд 2 |
|---|---|---|---|
| parking layout (352 lots, 17854 stalls) | 31.0 ms | 35.6 ms | 42.1 ms |
| map meshes: roads | 505.3 ms | 450.4 ms | 530.7 ms |
| map meshes: whole set | 505.5 ms | 450.9 ms | 530.8 ms |
| road meshing (близкая ступень) | 411.8 ms | 349.9 ms | 404.9 ms |
| road meshing (дальняя) | 347.5 ms | 304.1 ms | 354.2 ms |
| before the ribbons | 168.2 ms | 143.2 ms | 156.0 ms |
| warmup | 0.55 s | 0.51 s | 0.56 s |

Счётчики геометрии во всех трёх заездах бит-в-бит совпадают — поведение не
изменилось. Второй заезд ветки (на следующий день) совпал с master и даже чуть
медленнее первого заезда ветки: разброс между заездами **одного и того же кода**
(450 vs 531, ±9%) перекрывает любые различия ветка↔master. Итоговый вердикт:
**измеримой разницы нет** — «ветка на ~10% быстрее» из первого заезда была шумом.
Новой работы ни на кадр, ни на элемент загрузки изменение не добавляет.

Прямой запуск `target/debug/qwe` (без cargo) не работает: dynamic_linking-сборка
без LC_RPATH — `dyld: Library not loaded: @rpath/libstd-…dyld`. Замерный заезд —
только фоновым `cargo run` (тот факт, что он таск — не баг, а единственный способ).

## След сессии

- master fast-forward'нут на голову deep01 дважды: параллельная сессия в
  main checkout коммитила по ходу (5b6871f9 → e819c076), deep01 ребейзился на
  обе версии без конфликтов, второй ff лёг чисто (ed272973).
- **Конвенция для следующих заездов плана**: следующую сессию запускать через
  zcode-sessions `spawn_session` с `task: false` (указание владельца); порядок
  плана — №03 → №02 → №7 → №6 → №4 → №8/№5.

## Заметки

- Строки плана уехали от master (план писался раньше arch/* коммитов): копии сейчас
  parking.rs:173-179 + 233, roads.rs:987/1426/543, cars/mod.rs:230/384/477, wagons.rs:217/249.
- Тесты предикатов — в model/tests.rs (там живут carves_navmesh-тесты); запрос — на
  парс-фикстуре Overpass (parse/rails/tests.rs стиль).
- rail.rs::crossable удаляется вовсе: три места (roads.rs:82 import, roads.rs:1434,
  parse/rails.rs:196+37) переходят на road.is_at_grade(); rail/tests.rs:539-571
  переименовать упоминания.
- Урок прошлой сессии (актуален): never pass .md to check.sh; never edit while
  check.sh runs; pre-commit check через --all-targets незачем — check.sh сам
  гоняет test+clippy.

---

# NOTES — improve-architecture-sizes

Worktree for the remaining menu of the split-giants plan (qwe-split-giants-plan).
Rules as in phase A: pure code movement, check.sh green per commit, test names
invariant (baseline: 1517 tests, /tmp/qwe-tests-baseline.txt), interface paths
frozen via root re-exports, docs-twin in the same change.

Lessons so far: pre-commit check must be `cargo check --all-targets` (lib-test
consumers via `use super::*` are invisible to a plain check); test-consumer
names that rode the root scope come back as `#[cfg(test)]` re-exports in the
root (turning_radius, RoadNodeKind, Fortresses); never edit files while a
check.sh is running.

## Stage a — phase B: spread build_roads inline blocks

Wave 1 done (nested fns → subject files, bodies unchanged):
bece5505 merge edges → merges.rs; bfd60b95 kerb pockets → pockets.rs
(+POCKET_OVERLAP); 6753ef0c+55e7118a turning circles → junctions.rs
(+turning_radius/TURNING_CIRCLE_*, test re-exports fix); a5eb1925 seam dash
gaps → bridges.rs; d3a30f45 ring island bands + cover → rings.rs; 030f64b7
slit aprons → aprons.rs; 1e547f17 scrap phases → scraps.rs; 2bb2e6ab junction
paint → node_paint.rs; ff437221 tram bands → tram_band.rs (+DECK_CUT_SLACK;
TRAM_BAND_COLOR stays in root — tests read it there); 95cace3c wall ribbons →
walls.rs (+Fortresses test re-export).

Wave 2 (inline families → named subject fns, same captured values as params):
bcdfe21a merge ramps → merges.rs; 58ef8e29 plaza/road-island fills →
islands.rs (`lay_areas` returns RoadIslands + Bites); f23cdac1 ring approach
flags → rings.rs; 5adac231 footbridge flares → footbridge.rs (`lay_flares`);
4d01d53d verge side + bites → verges.rs::street_verged + scraps Bites::
on_points; 7c892c10 taper wedges → tapers.rs (wedge_run/push_asphalt/
push_sidewalks, #[allow(too_many_arguments)] per repo idiom; bf39d589 restored
the allowance that drifted onto street_verged); 88ae2ea5 deck seam aprons →
Bridges::push_seam_aprons.

**Stage a closed.** roads.rs 117.9K → 86.9K (2280 → 1688 lines): 12 nested
phases and 7 inline families live in their subject files; the main ribbon
loop, medians wiring and layer assembly stay in build_roads per its
docstring contract. check.sh green on every commit; test list after stage a
identical to baseline (1517 names). Perf verdict: pure code movement, nothing
new per frame or per load.

Remaining wave-2 residue (kept deliberately): bridge curb_wedges computation
inside the deck branch, the medians wiring, the loop's seam/trim wiring —
shared-state wiring per the contract.

## Stage b — CONTEXT.md diet — done (18941d2e)

docs/context-diet-plan.md is an inventory; the execution model: tiered glossary,
not a blind split. CONTEXT.md 213.5K → 107.5K: the "OSM map pipeline" section
156.5K → ~50K (every entry compressed to term + invariants + pointer to its
osm-map reference; all bold cross-reference titles preserved). The head and
tail sections were already at the charter's 5–10-line budget and stay.
Mechanism text was verified to live in the skill references before cutting
(spot-checked: beige, contact skirt, chequerboard, balconies_fit, boxless,
straighten_tails, green_aprons, brand band, MIN_ROW_RUN, fan_head).
check.sh green (note: never pass a .md to check.sh — its fmt step runs rustfmt
on it; run it with no args for docs-only commits).

## Stage c — dense subject files >60K

Done so far (all check.sh-green, test lists identical to baseline):
- db1c4a23 gores mouths → gores/mouths.rs (10.5K), gores.rs 66.9K → 50.3K
- 2859dfe1 gores splitters → gores/splitters.rs (7.8K)
- 1c150904 tags buildings family → tags/buildings.rs (488 lines), tags.rs
  75.2K → ~44K. Re-exports at tags root keep parse consumers' paths frozen;
  building_height/colour re-exports are #[cfg(test)] (only parse/tests.rs
  reads them). BUILDING_HEIGHT_RANGE stayed in tags root (structure_height
  reads it too); buildings.rs reaches it via `use super::`.
- cdb8dade lots tests → lots/tests.rs; lots.rs 68.8K → 42.8K (pockets
  precedent: tests in a child file, path unchanged)
- 1d8982b6 pairs.rs 89.5K → 40.2K + pairs/search.rs (Поиск: new, push_run,
  beside, Tracks, RunKind) + pairs/align.rs (Общая ось: align,
  follow_moved_nodes, seam machinery); pub consts stayed root, children
  `use super::` them; pairable re-exported `pub use self::search::pairable`
- a6726da3 paint.rs 115K → 27.8K hub + paint/{frames,spans,ground,material,
  painter}.rs. Root keeps shared consts, RoadPaintStyle, LineKind/LineBreaks,
  ArrowMarks+ARROW consts (node_paint reads the fields), MergeRamp (private
  methods used by painter), axis_offset, Station, PaintTag. paint/tests.rs got
  its own import block for the moved names; Painter fields pub(super).
- 1092fc67 node_paint.rs 128.2K → 43.3K + node_paint/{cluster,zebras,ends}.rs.
  cluster.rs holds paint_cluster (~45K — one closure environment, unsplittable
  without redesign; 60.2K, right at the limit) + edge machinery + Context;
  zebras.rs plank joins; ends.rs spills/short runs. NodePaint fields are
  root-private and children reach them (descendant visibility).

Commit messages: "split X into ... subjects" / "move ... tests into ...".
Sizes verified with wc -c after each split.

Remaining in stage c:
- parse/tests.rs 175.7K — c4, mapped at 1092fc67: 109 tests, families beside
  their passes: rails 94-196; verges 1578-1701 + probes 1520-1577; sidewalks
  1177-1269 + infer 1378-1433 + helpers 1358-1377 + 1423 (infer_sidewalks
  lives in sidewalks.rs); pavements 1270-1357; faiths 2286-2572; squaring
  2616-2892 + 3481-3542 + 3784-3857; house_pull 2893-3062 + 3594-3650;
  block_pull 3063-3186 + 3234-3360 + 3651-3783 + 3906-4067; tags family:
  measures 844-858, tags() helper 859-868 → parse/fixtures.rs (16 uses),
  path width 1019-1051, heights 1747-1978, kremlin/use/shop 2052-2214,
  fortress 2573-2615, colours 3361-3413, WESTERN_CHURCH 26-32 (only
  building_use test reads it); lots parse-level 358-428 → lots/tests.rs.
  Line numbers valid at 1092fc67 — recheck after each cut. After the moves,
  tests.rs keeps seam/trees/areas/roads-core/entrances (~100K) — split
  further into tests/ children by subject if it stays >60K.

Then stage d (#[ignore] diagnostics ~530 lines in roads tests — plan
recommendation: leave as is, only the author decides), then per user:
rebase onto master, merge the branch, remove the worktree.

Handoff facts: branch improve-architecture-sizes at 1092fc67 (32 commits
ahead of master 828a6853), worktree
.claude/worktrees/improve-architecture-sizes, target/ seeded, check.sh green
throughout, test list identical to baseline (/tmp/qwe-baseline-sorted.txt is
the sorted raw-format baseline, 1531 lines with `: test` suffix). Artifact
plan updated for stage a only
(https://artifacts.spacehubstore.com/a/qwe/qwe-split-giants-plan — NEW url;
the update endpoint on the legacy name 409s at any rev, worked around by
rename to -prev + save under project "qwe"; stage b/c updates still pending).

Lessons (see top): --all-targets before commit; test-consumer names come back
as cfg(test) re-exports; never edit while check.sh runs; never pass .md to
check.sh; `tags.rs` + `tags/` subdir coexist — mkdir before writing a child
module file; python line-split indexing is 0-based, file line N is lines[N-1]
(count the boundaries, don't trust memory); rustfmt runs at the END of
check.sh even when the build failed — re-check the file before patching a
failed split, multi-line replace patterns die on the reflowed text; the
recurring failure is a range that fell out of ALL files — the compiler catches
it, recover the block from `git show HEAD:<file>`; guard-bash denies a whole
command if any part pipes cargo — run the python patch and the check as
separate calls.
## Stage d — #[ignore] diagnostics fate

Left as is. Both are DIAG-TEMP harnesses driven by env vars (QWE_DIAG_JSON=...
in roads/tests.rs and roads/drawn/tests.rs) — author tooling, not rot. The
plan's recommendation stands: only the author decides.

## Rebase and merge — DONE via squash

master crossed the branch with bevy 0.20 / i_overlay 9 / rand 0.10 and doc
retargets (13 overlapping files). A per-commit rebase would resolve the same
API conflicts 35 times, so the branch was squashed onto master instead:
- improve-architecture-sizes-pre-rebase keeps all 35 original commits;
- improve-architecture-sizes = master 09d8bec0 + one squash commit (8ee63d30)
  + the API port (80486a4e: i_overlay mesh paths moved under mesh::float,
  Material2d::specialize takes the pipeline);
- check.sh OK after a cold 0.20 build: 1490 passed + 3 ignored, and the bare
  test names are identical to master's own list (1545 = 1545), so the squash
  lost nothing from master's new inspector and osm-id tests.
