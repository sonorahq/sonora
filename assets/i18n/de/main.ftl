# common
common-on = An
common-off = Aus
common-left = Links
common-right = Rechts
common-search = Suchen
common-unknown = Unbekannt
common-not-provided = Nicht angegeben
common-not-available = Nicht verfügbar
common-cancel = Abbrechen
common-save = Speichern
common-delete = Löschen
common-play = Abspielen
common-more = Mehr
common-previous = Zurück
common-next = Weiter
common-dismiss = Schließen
common-clear = Leeren
number-group = { "." }

# navigation
nav-history = Wiedergabeverlauf
nav-home = Start
nav-search = Suche
nav-library = Deine Bibliothek
nav-settings = Einstellungen
nav-songs = Songs
nav-albums = Alben
nav-playlists = Playlists
nav-artists = Künstler
nav-local = Lokale Musik
nav-back = Zurück
nav-forward = Vorwärts
nav-sidebar = Seitenleiste umschalten
nav-sidebar-right = Songtext und Warteschlange ein- oder ausblenden
nav-pinned = Angeheftet
nav-unpin = Lösen
nav-pin-hint = Zum Anheften hier ablegen
library-liked-songs = Favoriten
library-play-liked-songs = Abspielen
library-no-songs = Noch keine Favoriten
library-no-albums = Noch keine gespeicherten Alben
library-no-playlists = Noch keine Playlists
library-no-local-songs = Keine importierten Songs gefunden
library-no-local-albums = Keine importierten Alben gefunden
library-no-local-artists = Keine importierten Künstler gefunden
library-no-local-playlists = Noch keine lokalen Playlists
library-no-matches = Keine Treffer
library-not-loaded = Deine Bibliothek wurde nicht geladen
library-part-not-loaded = Dieser Teil deiner Bibliothek wurde nicht geladen
library-local-unconfigured = Richte deine lokale Bibliothek ein

# app menu
app-refresh-library = Bibliothek aktualisieren
app-sign-out = Abmelden
app-quit = Beenden
app-settings = Einstellungen …
app-hide = Sonora ausblenden
app-hide-others = Andere ausblenden
app-show-all = Alle einblenden
app-edit = Bearbeiten
app-cut = Ausschneiden
app-copy = Kopieren
app-paste = Einsetzen
app-select-all = Alles auswählen
app-window = Fenster
app-close-window = Fenster schließen
app-minimize = Im Dock ablegen
app-zoom = Zoomen

# tray menu
tray-show = Sonora anzeigen
tray-play = Wiedergabe
tray-pause = Pause

# table columns
column-played-at = Abgespielt
column-index = #
column-title = Titel
column-artist = Künstler
column-album = Album
column-date-added = Hinzugefügt am
column-added-by = Hinzugefügt von
column-modified = Geändert
column-length = Länge
column-plays = Wiedergaben
column-name = Name
column-owner = Ersteller
column-year = Jahr
column-tracks = Titel

# track menu
menu-add-to-playlist = Zur Playlist hinzufügen
menu-add-tracks-to-playlist = { $count ->
    [one] { $count } Track zur Playlist hinzufügen
   *[other] { $count } Tracks zur Playlist hinzufügen
}
menu-new-playlist = Neue Playlist
menu-edit-tags = Tags bearbeiten
menu-no-playlists = Keine Playlists
menu-add-to-library = Zu Favoriten hinzufügen
menu-add-tracks-to-library = { $count ->
    [one] { $count } Track zu Favoriten hinzufügen
   *[other] { $count } Tracks zu Favoriten hinzufügen
}
menu-remove-from-library = Aus Favoriten entfernen
menu-remove-tracks-from-library = { $count ->
    [one] { $count } Track aus Favoriten entfernen
   *[other] { $count } Tracks aus Favoriten entfernen
}
menu-remove-from-playlist = Aus der Playlist entfernen
menu-remove-tracks-from-playlist = { $count ->
    [one] { $count } Track aus Playlist entfernen
   *[other] { $count } Tracks aus Playlist entfernen
}
menu-remove-from-history = Aus dem Verlauf entfernen
menu-remove-tracks-from-history = { $count ->
    [one] { $count } Track aus Verlauf entfernen
   *[other] { $count } Tracks aus Verlauf entfernen
}
menu-play-next = Als Nächstes spielen
menu-play-tracks-next = { $count ->
    [one] { $count } Track als Nächstes spielen
   *[other] { $count } Tracks als Nächstes spielen
}
menu-add-to-queue = Zur Warteschlange hinzufügen
menu-add-tracks-to-queue = { $count ->
    [one] { $count } Track zur Warteschlange hinzufügen
   *[other] { $count } Tracks zur Warteschlange hinzufügen
}
menu-song-radio = Song-Radio öffnen
menu-go-to-album = Zum Album
menu-go-to-artist = Zum Künstler
menu-view-details = Details anzeigen
menu-copy-link = Link kopieren
menu-cut = Ausschneiden
menu-copy = Kopieren
menu-paste = Einfügen
menu-select-all = Alle auswählen
menu-remove-from-queue = Aus der Warteschlange entfernen
menu-open-playlist = Playlist öffnen
menu-play-playlist = Playlist abspielen
menu-rename-playlist = Playlist umbenennen
menu-delete-playlist = Playlist löschen
menu-add-playlist-to-library = Zur Bibliothek hinzufügen
menu-remove-playlist-from-library = Aus der Bibliothek entfernen
menu-make-playlist-public = Öffentlich machen
menu-make-playlist-private = Privat machen
menu-open-album = Album öffnen
menu-play-album = Album abspielen
menu-play-artist = Künstler abspielen

# playlist editor
playlist-name-placeholder = Name der Playlist
playlist-create-title = Playlist erstellen
playlist-rename-title = Playlist umbenennen
playlist-delete-title = Playlist löschen
playlist-delete-confirm = „{ $name }“ löschen? Das lässt sich nicht rückgängig machen.
playlist-again-title = Noch einmal hinzufügen?
playlist-again-confirm = Dieser Titel ist schon in „{ $name }“. Noch ein Mal hinzufügen?
playlist-again-add = Erneut hinzufügen

# confirm
confirm-remove-library-title = Von Bibliothek entfernen
confirm-remove-playlist-title = Von Playlist entfernen
confirm-remove-history-title = Von Verlauf entfernen
confirm-remove-songs = { $count ->
    [one] Den Song von der Bibliothek entfernen?
   *[other] { $count } Songs von der Bibliothek entfernen?
}
confirm-remove-playlist-songs = { $count ->
    [one] Den Song von der Playlist entfernen?
   *[other] { $count } Songs von der Playlist entfernen?
}
confirm-remove-history-songs = { $count ->
    [one] Den Song von dem Verlauf entfernen?
   *[other] { $count } Songs von dem Verlauf entfernen?
}
confirm-remove-albums = { $count ->
    [one] Den Album von der Bibliothek entfernen?
   *[other] { $count } Alben von der Bibliothek entfernen?
}
confirm-remove-playlists = { $count ->
    [one] Den Playlist von der Bibliothek entfernen?
   *[other] { $count } Playlists von der Bibliothek entfernen?
}

# queue panel
queue-title = Warteschlange
queue-history = Verlauf
queue-now-playing = Läuft gerade
queue-from = Aus
queue-up-next = Als Nächstes
queue-reset = Zurücksetzen
queue-clear = Leeren
queue-empty = Deine Warteschlange ist leer
queue-similar = Ähnliche Titel
queue-radio = Ähnliche Titel automatisch abspielen

# player bar
player-nothing-playing = Nichts wird abgespielt
player-percent = { $value } %
player-shuffle = Zufallswiedergabe
player-repeat = Wiederholen
player-repeat-all = Alle wiederholen
player-repeat-one = Titel wiederholen
player-mute = Stummschalten
player-unmute = Ton einschalten
player-previous = Vorheriger Titel
player-next = Nächster Titel
player-fullscreen = Vollbild
player-fullscreen-leave = Vollbild verlassen
fullscreen-artwork = Cover

# filters
filter-history = Wiedergabeverlauf filtern
history-empty = Abgespielte Titel erscheinen hier.
history-not-loaded = Der Wiedergabeverlauf konnte nicht geladen werden.
history-clear = Verlauf löschen
history-clear-title = Wiedergabeverlauf löschen
history-clear-confirm = Alle Wiedergaben werden von diesem Gerät entfernt. Das lässt sich nicht rückgängig machen.
filter-library = Bibliothek filtern
filter-album = Albumtitel filtern
filter-reset = Filter zurücksetzen
filter-duration = Dauer
filter-year = Jahr
filter-explicit = Nur explizite
filter-playable = Nur abspielbare
filter-owned = Von dir

# view
view-list = Liste
view-cards = Karten

# toolbar
tool-columns = Spalten
tool-sort = Sortieren
tool-filters = Filter

# login
login-signed-out = Melde dich an, um deine Musikbibliothek zu laden
login-restoring = Gespeicherte Sitzung wird geprüft…
login-authorizing = Warten auf die Bestätigung im Browser…
login-signed-in = Angemeldet als { $name }
login-failed-title = Anmeldung fehlgeschlagen
login-problem-region = Spotify öffnet keine Sitzung aus dem Land, in dem du dich befindest. Melde dich aus deinem Heimatland an oder ändere das Land in deinem Spotify-Konto.
login-problem-credentials = Deine gespeicherte Spotify-Sitzung ist nicht mehr gültig. Melde dich erneut an, um fortzufahren.
login-problem-network = Sonora konnte Spotify nicht erreichen. Prüfe deine Internetverbindung und versuche es erneut.
login-problem-cancelled = Du hast die Browserseite geschlossen, bevor die Anmeldung bestätigt war. Fang noch einmal an, um sie abzuschließen.
login-problem-refused = Spotify hat die Anmeldung abgelehnt. Warte einen Moment und versuche es erneut.
login-problem-premium = Sonora streamt über Spotify Premium, und dieses Konto hat es nicht. Melde dich mit einem Premium-Konto an, um fortzufahren.
login-sign-in = Mit { $provider } anmelden
login-connect-cookies = Cookies manuell einfügen
login-cookie-open = YouTube Music öffnen
login-cookie-submit = Weiter
login-cookie-hint = Füge hier den Cookie-Request-Header ein
login-cookie-step-1 = Öffne music.youtube.com und stelle sicher, dass du angemeldet bist. Am besten funktioniert ein Inkognitofenster.
login-cookie-step-2 = Drücke F12, öffne den Tab „Netzwerkanalyse“ und lade die Seite neu.
login-cookie-step-3 = Wähle eine Anfrage namens „browse“ oder „next“.
login-cookie-step-4 = Suche unter „Kopfzeilen“ bei den Anfrage-Headern den Eintrag „Cookie“, klicke ihn mit der rechten Maustaste an und kopiere seinen Wert.
login-cookie-step-note = Füge den vollständigen Wert unten ein: die Cookie-Ansicht der Anfrage reicht nicht, weil der Wert SAPISID und __Secure-3PAPISID enthalten muss.
login-cookie-title = Füge deine YouTube-Music-Cookies ein, um die Anmeldung abzuschließen
login-window-title = Bei { $provider } anmelden
login-use = { $provider } verwenden
login-guest-title = Gastmodus
login-guest-use = Gastmodus verwenden
login-guest-detail = Stöbern und abspielen ohne Konto. Bibliothek, Favoriten und Playlists bleiben außen vor.
login-usage-consent = Hilf uns zu schätzen, wie viele Menschen Sonora nutzen.
login-device-code = Gib diesen Code auf { $url } ein
login-server-title = Mit deinem Subsonic-Server verbinden
login-server-detail = Gib die Adresse eines beliebigen Subsonic- oder OpenSubsonic-Servers ein (Navidrome, Airsonic, Gonic, …) und melde dich mit deinem Server-Benutzernamen und -Passwort an. Die Sitzung bleibt auf diesem Gerät.
login-server-hint = https://music.example.com
login-username-hint = Benutzername
login-password-hint = Passwort
login-server-submit = Verbinden
login-account-title = Konto auswählen
login-account-detail = Diese Sitzung ist bei mehreren Google-Konten angemeldet. Wähle das Konto, das Sonora verwenden soll.

# album and playlist pages
detail-album = Album
detail-playlist = Playlist
detail-play-album = Album abspielen
detail-play-playlist = Playlist abspielen

# play button
play-pause = Pause
play-resume = Fortsetzen
play-loading = Wird geladen…
play-shuffle = Zufallswiedergabe

# artist page
artist-eyebrow = Künstler
artist-monthly-listeners = { $count ->
    [one] { $value } monatlicher Hörer
   *[other] { $value } monatliche Hörer
}
artist-play = Jetzt abspielen
artist-popular = Beliebt
artist-popular-eyebrow = Diesen Künstler entdecken
artist-popular-empty = Von diesem Künstler gibt es noch nichts zum Abspielen
artist-popular-more = Alle anzeigen
artist-popular-less = Weniger anzeigen
artist-releases = Veröffentlichungen
artist-releases-more = Alle anzeigen
artist-releases-less = Weniger anzeigen
artist-filter-all = Alle
artist-filter-albums = Alben
artist-filter-singles = Singles
artist-filter-eps = EPs

# user profile page
user-eyebrow = Profil
user-followers = { $count ->
    [one] { $value } Follower
   *[other] { $value } Follower
}
user-following = { $count ->
   *[other] Folgt { $value }
}
user-playlists = Öffentliche Playlists
user-playlists-empty = Noch keine öffentlichen Playlists

# release kinds
release-album = Album
release-single = Single
release-compilation = Kompilation
release-ep = EP
release-audiobook = Hörbuch
release-podcast = Podcast
release-meta = { $year } • { $kind }

# home page
home-quick-picks = Schnellauswahl
home-listen-again = Noch einmal anhören
home-quick-picks-eyebrow = Mit einem Song starten
home-quick-picks-empty = Markiere ein paar Songs als Favoriten, dann erscheinen sie hier

# search page
search-placeholder = Was möchtest du hören?
search-browse = Alles durchsuchen
genre-empty = Hier gibt es noch nichts zu sehen
search-best-match = Beste Übereinstimmung
search-no-matches = Keine Treffer
search-results = Ergebnisse
search-songs = Songs
search-artists = Künstler
search-albums-playlists = Alben & Playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } Song in der Bibliothek
       *[other] { $count } Songs in der Bibliothek
    }
kind-song = Song
kind-artist = Künstler
kind-album = Album
kind-playlist = Playlist

# song page
song-eyebrow = Song
song-play = Song abspielen
song-view-album = Album anzeigen
song-loading = Songinformationen werden geladen…
song-about = Über diesen Song
song-album = Album
song-released = Veröffentlicht
song-streams = Streams
song-position = Position
song-label = Label
song-popularity = Beliebtheit
song-popularity-value = { $value } %
song-disc-track = CD { $disc }, Titel { $track }
song-track = Titel { $track }
song-credits = Mitwirkende
song-performed-by = Interpretiert von
song-details = Genres & Details
song-genres = Genres
song-language = Sprache
song-content = Inhalt
song-explicit = Explizit
song-clean = Jugendfrei
artist-about = Über den Künstler
artist-about-fallback = Entdecke die beliebten Songs und Veröffentlichungen des Künstlers.
artist-about-open = Zum Künstler
song-copyright = © { $notice }

# song languages
language-ar = Arabisch
language-de = Deutsch
language-en = Englisch
language-es = Spanisch
language-fr = Französisch
language-hi = Hindi
language-it = Italienisch
language-ja = Japanisch
language-ko = Koreanisch
language-pt = Portugiesisch
language-ru = Russisch
language-tr = Türkisch
language-uk = Ukrainisch
language-zh = Chinesisch
language-zxx = Kein sprachlicher Inhalt

# counts
count-songs =
    { $count ->
        [one] { $count } Song
       *[other] { $count } Songs
    }
count-tracks =
    { $count ->
        [one] { $count } Titel
       *[other] { $count } Titel
    }

# dates
date-just-now = Gerade eben
date-minute-ago = Vor einer Minute
date-minutes-ago = Vor { $count } Minuten
date-today = Heute um { $time }
date-yesterday = Gestern um { $time }
date-time = { $date }, { $time }
date-full = { $day }. { $month } { $year }
month-1 = Jan.
month-2 = Feb.
month-3 = März
month-4 = Apr.
month-5 = Mai
month-6 = Juni
month-7 = Juli
month-8 = Aug.
month-9 = Sept.
month-10 = Okt.
month-11 = Nov.
month-12 = Dez.

# settings
settings-tab-general = Allgemein
settings-tab-appearance = Erscheinungsbild
settings-tab-playback = Wiedergabe
settings-tab-privacy = Privatsphäre
settings-theme = Design
settings-theme-detail = Wähle die Farbpalette der Anwendung
settings-opacity = Deckkraft
settings-opacity-detail = Deckkraft des App-Hintergrunds anpassen
settings-opacity-value = { $percent } %
settings-theme-config = Konfiguration öffnen
settings-adaptive = Adaptives Design
settings-adaptive-detail = Färbt die Palette nach dem Cover des laufenden Albums
settings-visualizer = Visualizer
settings-visualizer-detail = Spektrum hinter Vollbildcover anzeigen
settings-icons = Symbolsatz
settings-icons-detail = Wähle den Symbolsatz für die Oberfläche
settings-motion = Bewegung reduzieren
settings-motion-detail = Animationen und Übergänge der Oberfläche überspringen
settings-pace = Animationsgeschwindigkeit
settings-pace-detail = Wie schnell Animationen der Oberfläche ablaufen
settings-saver = Energiesparen
settings-saver-detail = Bildrate der Animationen begrenzen, wenn Sonora nicht im Fokus ist, ab dem nächsten Start
settings-corners = Ecken
settings-corners-detail = Wie stark Flächen und Bedienelemente abgerundet sind
settings-font = Schriftgröße
settings-font-detail = Basisgröße des Textes, alles andere skaliert mit
settings-font-value = { $size } px
settings-startup = Beim Start anzeigen
settings-startup-detail = Der Bildschirm, mit dem Sonora startet
settings-entries = Einträge der Seitenleiste
settings-entries-detail = Die Bereiche, die in der Seitenleiste erscheinen
settings-entries-pick = Einträge wählen
settings-language = Sprache
settings-language-detail = Die Sprache, die Sonora in der Oberfläche verwendet
settings-language-system = System
settings-language-search = Sprache suchen
settings-language-none = Keine Sprache gefunden
settings-typeface = Schriftart
settings-typeface-detail = Die Schrift, die Sonora in der Oberfläche verwendet
settings-typeface-system = Standard
settings-typeface-search = Schriftart suchen
settings-typeface-none = Keine Schriftart gefunden
settings-server-side-decorations = Serverseitige Fensterdekorationen
settings-server-side-decorations-detail = Titelleiste, Rahmen und Schatten vom Compositor zeichnen lassen
settings-typeface-loading = Wird geladen…
settings-window-controls = Fenstersteuerung
settings-window-controls-detail = Minimieren, Maximieren und Schließen in der Titelleiste zeichnen
settings-traffic-light-controls = Steuerung im Ampel-Stil
settings-traffic-light-controls-detail = Minimieren, Maximieren und Schließen als farbige Punkte zeichnen
settings-window-rounding = Fensterecken
settings-window-rounding-detail = Wie stark die Fensterecken abgerundet sind
settings-controls-side = Seite der Steuerung
settings-controls-side-detail = An welchem Ende der Titelleiste die Bedienelemente sitzen
settings-close-to-tray = Beim Schließen weiterspielen
settings-close-to-tray-detail = Sonora bleibt nach dem Schließen im System-Tray und spielt weiter
settings-normalisation = Lautstärke angleichen
settings-normalisation-detail = Hält Titel auf einer gleichmäßigen Lautstärke
settings-gapless = Lückenlose Wiedergabe
settings-gapless-detail = Lässt einen Titel ohne Pause in den nächsten laufen, so wie das Album gedacht war
settings-panel-lyrics-size = Lyricsgröße (panel)
settings-panel-lyrics-size-detail = Größe des Lyrics-Textes in der Seitenleiste, auf der Basis der Basis-Schriftgröße
settings-fullscreen-lyrics-size = Lyricsgröße (fullscreen)
settings-fullscreen-lyrics-size-detail = Größe des Lyrics-Textes auf dem Vollbildplayer, auf der Basis der Basis-Schriftgröße
settings-lyrics-size-value = { $size }%
settings-lyrics-for-local-files = Lyrics für lokale Dateien
settings-lyrics-for-local-files-detail = Metadaten aus lokalen Dateien verwenden, um Lyrics vom Internet zu laden
settings-karaoke-lyrics = Karaoke-Songtext
settings-karaoke-lyrics-detail = Den Songtext Wort für Wort hervorheben, wenn Timings vorliegen
settings-blur-lyrics = Interaktive Lyrics verwischen
settings-blur-lyrics-detail = Nächste und vorherige Lyrics verwischen
settings-romanized-lyrics = Romanisierter Songtext
settings-romanized-lyrics-detail = Lokal erzeugte Aussprache für ausgewählte Schriftsysteme anzeigen
settings-romanization-writing-systems = Schriftsysteme
settings-romanization-japanese = Japanische Schrift
settings-romanization-chinese = Chinesische Schrift
settings-romanization-korean = Koreanische Schrift
settings-romanization-cyrillic = Kyrillisch
settings-romanization-greek = Griechische Schrift
settings-romanization-arabic = Arabische Schrift
settings-romanization-other = Andere Schriftsysteme
settings-advanced = Erweitert
settings-group-window = Fenster
settings-group-accounts = Konten
settings-group-library = Bibliothek
settings-group-text = Text
settings-group-motion = Bewegung
settings-group-title-bar = Titelleiste
settings-group-window-style = Fensterstil
settings-group-lyrics = Songtext
settings-group-project = Projekt
settings-adaptive-menu = Adaptives Kontextmenü
settings-adaptive-menu-detail = Lässt Einträge weg, die die Zeile ohnehin zeigt, etwa das Album oder den Künstler
settings-accounts = Konten verwalten
settings-accounts-detail = Die Dienste, von denen dieses Gerät abspielen kann
settings-provider-none = Nicht verbunden
settings-provider-connected = Verbunden
settings-provider-current = Wiedergabe über diesen Dienst
settings-provider-guest = Wiedergabe als Gast
settings-provider-switch = Wechseln zu
settings-sign-out = Abmelden
settings-local-folder = Ordner mit importierter Musik
settings-local-folder-empty = Nicht eingerichtet
settings-choose-folder = Ordner wählen…
settings-rescan = Neu einlesen
settings-tab-about = Über
settings-version = Version
settings-version-detail = Der Build von Sonora, den du verwendest
settings-license = Lizenz
settings-license-detail = GNU General Public License Version 3 oder später
settings-license-view = Lizenz lesen
settings-source = Quellcode
settings-source-detail = Der zu diesem Build gehörende Quellcode
settings-source-view = Repository öffnen
settings-team = Team
settings-team-github = GitHub
settings-role-lead-maintainer = Leitender Betreuer
settings-role-maintainer = Betreuer
settings-role-contributor = Mitwirkender
settings-notice = Copyright © 2026 Sonora Contributors. Sonora kommt ohne jede Gewährleistung. Es ist freie Software, und du darfst es unter den Bedingungen der GNU General Public License Version 3 oder später weitergeben. Sonora ist inoffiziell und steht in keiner Verbindung zu Spotify AB.

# themes
theme-system = System
theme-dark = Dunkel
theme-light = Hell
theme-midnight = Mitternacht
theme-forest = Wald
theme-ocean = Ozean
theme-rose = Rosé
theme-lavender = Lavendel
theme-amber = Bernstein

# corners
corners-square = Eckig
corners-subtle = Dezent
corners-rounded = Abgerundet
corners-round = Rund

# motion
motion-system = Wie im System
motion-always = Immer
motion-never = Nie
pace-slow = Langsam
pace-base = Standard
pace-quick = Schnell
saver-off = Aus
saver-light = Leicht ({ $fps } FPS)
saver-medium = Mittel ({ $fps } FPS)
saver-strong = Stark ({ $fps } FPS)

toast-playlist-created = Playlist erstellt
toast-playlist-renamed = Playlist umbenannt
toast-playlist-deleted = Playlist gelöscht
toast-playlist-added = Playlist zur Bibliothek hinzugefügt
toast-playlist-removed = Playlist aus der Bibliothek entfernt
toast-playlist-visibility = Sichtbarkeit der Playlist geändert
toast-track-added = Zu { $name } hinzugefügt
toast-track-removed = Aus { $name } entfernt
toast-playlist-failed = Diese Änderung konnte nicht gespeichert werden
toast-playlist-busy = Eine andere Änderung läuft noch
toast-playlist-signed-out = Melde dich an, um Playlists zu ändern
toast-queued-track = { $name } zur Warteschlange hinzugefügt
toast-next-track = { $name } läuft als Nächstes
toast-queued-album = Album zur Warteschlange hinzugefügt
toast-next-album = Album läuft als Nächstes
toast-queued-playlist = Playlist zur Warteschlange hinzugefügt
toast-next-playlist = Playlist läuft als Nächstes
toast-queued-artist = Künstler zur Warteschlange hinzugefügt
toast-next-artist = Künstler läuft als Nächstes
toast-queue-failed = Das konnte nicht zur Warteschlange hinzugefügt werden
toast-keys-refused = Spotify gibt diesem Konto keine Wiedergabeschlüssel
toast-sign-in-to-play = { $name } streamt nur für angemeldete Hörer
toast-track-unplayable = { $name } konnte nicht abgespielt werden
toast-library-add-failed = { $name } konnte nicht zur Mediathek hinzugefügt werden
toast-library-remove-failed = { $name } konnte nicht aus der Mediathek entfernt werden

# lyrics
lyrics-title = Songtext
lyrics-idle = Spiele etwas ab, um den Songtext zu sehen
lyrics-loading = Songtext wird gesucht…
lyrics-missing = Kein Songtext gefunden, sorry!
lyrics-instrumental = Dieser Titel ist instrumental
lyrics-failed = Der Songtext-Dienst war nicht erreichbar
lyrics-follow = Dem Song wieder folgen
lyrics-source = Songtext von { $source }
lyrics-writers = Geschrieben von { $writers }

update-available = Sonora { $version } ist da
update-detail = Du hast { $running }. Sieh dir an, was sich geändert hat, oder aktualisiere jetzt.
update-detail-notes = Du hast { $running }. Sieh dir an, was sich geändert hat, und aktualisiere Sonora so, wie du es installiert hast.
update-notes = Neuerungen
update-now = Aktualisieren
update-later = Später
update-working = Update wird geladen…
update-failed = Das Update konnte nicht installiert werden. Versuch es über die Releases-Seite.
settings-check-updates = Nach Updates suchen
settings-check-updates-detail = Beim Start einmal bei GitHub nachfragen, ob eine neuere Version da ist. Sonora installiert das Update nur unter Windows selbst, sonst zeigt es dir, was sich geändert hat

# tags
tags-edit-title = Tags bearbeiten
tags-sheet-song = Titel
tags-sheet-album = Album
tags-sheet-details = Details
tags-title = Titel
tags-artist = Künstler
tags-track = Titelnummer
tags-track-total = Titel auf dem Release
tags-disc = CD-Nummer
tags-disc-total = CDs im Release
tags-album = Album
tags-album-artist = Album-Künstler
tags-year = Jahr
tags-genre = Genre
tags-composer = Komponist
tags-publisher = Label
tags-isrc = ISRC
tags-comment = Kommentar
toast-tags-saved = Tags für { $name } gespeichert
toast-tags-failed = Die Tags konnten nicht gespeichert werden
