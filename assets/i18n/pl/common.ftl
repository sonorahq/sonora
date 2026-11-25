# common
common-left = Po lewej
common-right = Po prawej
common-search = Szukaj
common-unknown = Nieznane
common-not-provided = Nie podano
common-not-available = Brak danych
common-unavailable = { $name } (niedostępny)
common-cancel = Anuluj
common-save = Zapisz
common-delete = Usuń
common-play = Odtwórz
common-more = Więcej
common-previous = Wstecz
common-next = Dalej
common-dismiss = Zamknij
common-clear = Wyczyść
common-undo = Cofnij
common-redo = Ponów
common-cut = Wytnij
common-copy = Kopiuj
common-paste = Wklej
common-show-all = Pokaż wszystkie
common-show-less = Pokaż mniej
common-local-files = Pliki lokalne
common-server-address = Adres serwera
common-api-key = Klucz API
common-not-now = Nie teraz
common-not-connected = Niepodłączone
common-sonora = Sonora
common-more-count = +{ $count }
number-group = { "\u00A0" }

# units
unit-percent = { $value }%
unit-db = { $value } dB
unit-hz = { $value } Hz
unit-khz = { $value } kHz
unit-kbps = { $value } kb/s
unit-gb = { $value } GB
unit-mb = { $value } MB
unit-fps = { $value } FPS
unit-px = { $value } px
runtime-seconds = { $seconds } s
runtime-minutes = { $minutes } min { $seconds } s
runtime-hours = { $hours } h { $minutes } min

# navigation
nav-history = Historia
nav-home = Strona główna
nav-search = Szukaj
nav-library = Twoja biblioteka
nav-settings = Ustawienia
nav-songs = Utwory
nav-albums = Albumy
nav-playlists = Playlisty
nav-artists = Wykonawcy
nav-local = Muzyka lokalna
nav-back = Wstecz
nav-forward = Dalej
nav-sidebar = Pokaż lub ukryj panel boczny
nav-sidebar-right = Pokaż lub ukryj tekst i kolejkę
nav-pinned = Przypięte
nav-unpin = Odepnij
nav-pin-hint = Upuść tutaj, aby przypiąć
library-liked-songs = Ulubione
library-no-songs = Brak ulubionych
library-no-albums = Brak zapisanych albumów
library-no-playlists = Brak playlist
library-no-artists = Brak ulubionych wykonawców
library-no-local-songs = Nie znaleziono zaimportowanych utworów
library-no-local-albums = Nie znaleziono zaimportowanych albumów
library-no-local-artists = Nie znaleziono zaimportowanych wykonawców
library-no-local-playlists = Nie ma jeszcze lokalnych playlist
library-no-catalog-songs = Nie znaleziono utworów
library-no-catalog-albums = Nie znaleziono albumów
library-no-catalog-artists = Nie znaleziono wykonawców
library-no-matches = Brak wyników
library-not-loaded = Biblioteka się nie wczytała
library-scanning = Przeglądam twoje foldery…
library-scanning-progress = Czytam twoją muzykę… { $read } z { $found }
library-part-not-loaded = Ta część biblioteki się nie wczytała
library-local-unconfigured = Skonfiguruj lokalną bibliotekę

# app menu
app-refresh = Odśwież
app-sign-out = Wyloguj się
app-quit = Zakończ
app-settings = Ustawienia…
app-hide = Ukryj Sonora
app-hide-others = Ukryj pozostałe
app-show-all = Pokaż wszystkie
app-edit = Edycja
app-select-all = Zaznacz wszystko
app-window = Okno
app-close-window = Zamknij okno
app-minimize = Minimalizuj
app-zoom = Powiększ

# tray menu
tray-show = Pokaż Sonorę
tray-play = Odtwarzaj
tray-pause = Wstrzymaj

# table columns
column-played-at = Odtworzono
column-index = #
column-title = Tytuł
column-artist = Wykonawca
column-album = Album
column-genre = Gatunek
column-date-added = Data dodania
column-added-by = Dodane przez
column-modified = Zmodyfikowano
column-length = Czas
column-plays = Odtworzenia
column-name = Nazwa
column-owner = Właściciel
column-year = Rok
column-tracks = Utwory

# filters
filter-history = Filtruj historię słuchania
history-empty = Odtworzone utwory pojawią się tutaj.
history-not-loaded = Nie udało się wczytać historii słuchania.
history-clear = Wyczyść historię
history-clear-title = Wyczyść historię słuchania
history-clear-confirm = Wszystkie odtworzenia zostaną usunięte z tego urządzenia. Nie można tego cofnąć.
filter-library = Filtruj bibliotekę
filter-album = Filtruj utwory albumu
filter-reset = Wyczyść filtry
filter-duration = Czas trwania
filter-year = Rok
filter-hide-explicit = Ukryj z wulgaryzmami
filter-favorites = Tylko ulubione
filter-downloaded = Pobrane
filter-owned = Moje

# view
view-list = Lista
view-cards = Kafelki

# toolbar
tool-sort = Sortowanie
tool-filters = Filtry

# release kinds
release-album = Album
release-single = Singiel
release-compilation = Kompilacja
release-ep = EP
release-audiobook = Audiobook
release-podcast = Podcast
kind-song = Utwór
kind-artist = Wykonawca
kind-album = Album
kind-playlist = Playlista

# song languages
language-ar = Arabski
language-de = Niemiecki
language-en = Angielski
language-es = Hiszpański
language-fr = Francuski
language-hi = Hindi
language-it = Włoski
language-ja = Japoński
language-ko = Koreański
language-pt = Portugalski
language-ru = Rosyjski
language-tr = Turecki
language-uk = Ukraiński
language-zh = Chiński
language-zxx = Bez treści językowej

# counts
count-songs =
    { $count ->
        [one] { $count } utwór
        [few] { $count } utwory
       *[other] { $count } utworów
    }
count-tracks =
    { $count ->
        [one] { $count } utwór
        [few] { $count } utwory
       *[other] { $count } utworów
    }

# dates
date-just-now = Przed chwilą
date-minute-ago = Minutę temu
date-minutes-ago =
    { $count ->
        [one] { $count } minutę temu
        [few] { $count } minuty temu
       *[other] { $count } minut temu
    }
date-today = Dzisiaj o { $time }
date-yesterday = Wczoraj o { $time }
date-time = { $date }, { $time }
date-full = { $day } { $month } { $year }
month-1 = sty
month-2 = lut
month-3 = mar
month-4 = kwi
month-5 = maj
month-6 = cze
month-7 = lip
month-8 = sie
month-9 = wrz
month-10 = paź
month-11 = lis
month-12 = gru

nav-pin = Przypnij
nav-nothing-pinned = Tu jest pusto
nav-pins-alphabetical = Alfabetycznie
nav-pins-kind = Według typu
nav-show-full-library = Pokaż całą bibliotekę
nav-return-top = Do góry

# power
wake-reason = Odtwarzana jest muzyka
