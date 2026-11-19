# common
common-left = Слева
common-right = Справа
common-search = Поиск
common-unknown = Неизвестно
common-not-provided = Не указано
common-not-available = Нет данных
common-unavailable = { $name } (недоступна)
common-cancel = Отмена
common-save = Сохранить
common-delete = Удалить
common-play = Играть
common-more = Ещё
common-previous = Назад
common-next = Вперёд
common-dismiss = Закрыть
common-clear = Очистить
common-undo = Отменить
common-redo = Повторить
common-cut = Вырезать
common-copy = Копировать
common-paste = Вставить
common-show-all = Показать все
common-show-less = Показать меньше
common-local-files = Локальные файлы
common-server-address = Адрес сервера
common-api-key = API-ключ
common-not-now = Не сейчас
common-not-connected = Не подключено
common-sonora = Sonora
common-more-count = +{ $count }
number-group = { "\u00A0" }

# units
unit-percent = { $value }%
unit-db = { $value } дБ
unit-hz = { $value } Гц
unit-khz = { $value } кГц
unit-kbps = { $value } кбит/с
unit-gb = { $value } ГБ
unit-mb = { $value } МБ
unit-fps = { $value } FPS
unit-px = { $value } px
runtime-seconds = { $seconds } с
runtime-minutes = { $minutes } мин { $seconds } с
runtime-hours = { $hours } ч { $minutes } мин

# navigation
nav-history = История
nav-home = Главная
nav-search = Поиск
nav-library = Моя медиатека
sidebar-library = Медиатека
nav-settings = Настройки
account-manage = Управление аккаунтами
nav-songs = Треки
nav-albums = Альбомы
nav-playlists = Плейлисты
nav-artists = Исполнители
nav-local = Локальная музыка
nav-back = Назад
nav-forward = Вперёд
nav-sidebar = Показать или скрыть боковую панель
nav-sidebar-right = Показать или скрыть текст и очередь
nav-pinned = Закреплённое
nav-unpin = Открепить
nav-pin-hint = Перетащите сюда, чтобы закрепить
library-liked-songs = Избранное
library-no-songs = Пока нет избранного
library-no-albums = Пока нет сохранённых альбомов
library-no-playlists = Пока нет плейлистов
library-no-artists = Пока нет избранных исполнителей
library-no-local-songs = Импортированные треки не найдены
library-no-local-albums = Импортированные альбомы не найдены
library-no-local-artists = Импортированные исполнители не найдены
library-no-local-playlists = Локальных плейлистов пока нет
library-no-catalog-songs = Треки не найдены
library-no-catalog-albums = Альбомы не найдены
library-no-catalog-artists = Исполнители не найдены
library-no-matches = Ничего не найдено
library-not-loaded = Библиотека не загрузилась
library-scanning = Просматриваю ваши папки…
library-scanning-progress = Читаю вашу музыку… { $read } из { $found }
library-part-not-loaded = Этот раздел библиотеки не загрузился
library-local-unconfigured = Настройте локальную библиотеку

# app menu
app-refresh = Обновить
app-sign-out = Выйти
app-quit = Выход
app-settings = Настройки…
app-hide = Скрыть Sonora
app-hide-others = Скрыть остальные
app-show-all = Показать все
app-edit = Правка
app-select-all = Выбрать все
app-window = Окно
app-close-window = Закрыть окно
app-minimize = Свернуть
app-zoom = Изменить масштаб

# tray menu
tray-show = Показать Sonora
tray-play = Играть
tray-pause = Пауза

# table columns
column-played-at = Прослушано
column-index = #
column-title = Название
column-artist = Исполнитель
column-album = Альбом
column-genre = Жанр
column-date-added = Дата добавления
column-added-by = Добавил
column-modified = Изменён
column-length = Длительность
column-plays = Прослушивания
column-name = Название
column-owner = Владелец
column-year = Год
column-tracks = Треки

# filters
filter-history = Фильтр истории прослушиваний
history-empty = Прослушанные треки появятся здесь.
history-not-loaded = Не удалось загрузить историю прослушиваний.
history-clear = Очистить историю
history-clear-title = Очистить историю прослушиваний
history-clear-confirm = Все прослушивания будут удалены с этого устройства. Отменить это нельзя.
filter-library = Фильтр медиатеки
filter-album = Фильтр треков альбома
filter-reset = Сбросить фильтры
filter-duration = Длительность
filter-year = Год
filter-hide-explicit = Скрыть с ненормативной лексикой
filter-favorites = Только избранное
filter-downloaded = Скачанное
filter-owned = Ваши
filter-local-only = Только локальные
filter-streaming-only = Только стриминг

# view
view-list = Список
view-cards = Карточки

# toolbar
tool-sort = Сортировка
tool-filters = Фильтры
tool-account = Аккаунт

# release kinds
release-album = Альбом
release-single = Сингл
release-compilation = Сборник
release-ep = EP
release-audiobook = Аудиокнига
release-podcast = Подкаст
kind-song = Трек
kind-artist = Исполнитель
kind-album = Альбом
kind-playlist = Плейлист

# song languages
language-ar = Арабский
language-de = Немецкий
language-en = Английский
language-es = Испанский
language-fr = Французский
language-hi = Хинди
language-it = Итальянский
language-ja = Японский
language-ko = Корейский
language-pt = Португальский
language-ru = Русский
language-tr = Турецкий
language-uk = Украинский
language-zh = Китайский
language-zxx = Без слов

# counts
count-songs =
    { $count ->
        [one] { $count } трек
        [few] { $count } трека
       *[other] { $count } треков
    }
count-tracks =
    { $count ->
        [one] { $count } трек
        [few] { $count } трека
       *[other] { $count } треков
    }

# dates
date-just-now = Только что
date-minute-ago = Минуту назад
date-minutes-ago =
    { $count ->
        [one] { $count } минуту назад
        [few] { $count } минуты назад
       *[other] { $count } минут назад
    }
date-today = Сегодня в { $time }
date-yesterday = Вчера в { $time }
date-time = { $date }, { $time }
date-full = { $day } { $month } { $year }
month-1 = янв.
month-2 = фев.
month-3 = мар.
month-4 = апр.
month-5 = мая
month-6 = июн.
month-7 = июл.
month-8 = авг.
month-9 = сен.
month-10 = окт.
month-11 = ноя.
month-12 = дек.

nav-pin = Закрепить
nav-nothing-pinned = Здесь пусто
nav-pins-alphabetical = По алфавиту
nav-pins-kind = По типу
nav-show-full-library = Показывать всю медиатеку
nav-return-top = Наверх

# power
wake-reason = Играет музыка
