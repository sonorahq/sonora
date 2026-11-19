# login
login-restoring = Gespeicherte Sitzung wird geprüft…
login-authorizing = Warten auf die Bestätigung im Browser…
login-signed-in = Angemeldet als { $name }
login-failed-title = Anmeldung fehlgeschlagen
login-problem-region = Spotify öffnet keine Sitzung aus dem Land, in dem du dich befindest. Melde dich aus deinem Heimatland an oder ändere das Land in deinem Spotify-Konto.
login-problem-credentials = Deine gespeicherte Spotify-Sitzung ist nicht mehr gültig. Melde dich erneut an, um fortzufahren.
login-problem-network = Sonora konnte den Musikdienst nicht erreichen. Prüfe deine Internetverbindung und versuche es erneut.
login-problem-cancelled = Du hast die Browserseite geschlossen, bevor die Anmeldung bestätigt war. Fang noch einmal an, um sie abzuschließen.
login-problem-refused = Spotify hat die Anmeldung abgelehnt. Warte einen Moment und versuche es erneut.
login-problem-premium = Sonora streamt über Spotify Premium, und dieses Konto hat es nicht. Melde dich mit einem Premium-Konto an, um fortzufahren.
login-sign-in = Mit { $provider } anmelden
login-connect-cookies = Cookies manuell einfügen
login-cookie-open = { $provider } öffnen
login-cookie-submit = Weiter
login-cookie-hint = Füge hier den Cookie-Request-Header ein
login-cookie-step-1 = Öffne music.youtube.com und stelle sicher, dass du angemeldet bist. Am besten funktioniert ein Inkognitofenster.
login-cookie-step-2 = Drücke F12, öffne den Tab „Netzwerkanalyse“ und lade die Seite neu.
login-cookie-step-3 = Wähle eine Anfrage namens „browse“ oder „next“.
login-cookie-step-4 = Suche unter „Kopfzeilen“ bei den Anfrage-Headern den Eintrag „Cookie“, klicke ihn mit der rechten Maustaste an und kopiere seinen Wert.
login-cookie-step-note = Füge den vollständigen Wert unten ein: die Cookie-Ansicht der Anfrage reicht nicht, weil der Wert SAPISID und __Secure-3PAPISID enthalten muss.
login-cookie-header-title = Füge deine { $provider }-Cookies ein, um die Anmeldung abzuschließen
login-cookie-apple-step-3 = Wähle eine beliebige Anfrage an { $site }.
login-cookie-apple-note = Füge unbedingt den ganzen Wert ein, einschließlich { $cookie }.
login-cookie-named-title = Füge dein { $provider }-Cookie ein, um die Anmeldung abzuschließen
login-cookie-named-hint = Cookie-Wert hier einfügen
login-cookie-named-step-1 = Öffne { $site } und stelle sicher, dass du angemeldet bist.
login-cookie-named-step-2 = Drücke F12 und öffne in Firefox den Tab „Web-Speicher“ oder in Chrome den Tab „Anwendung“.
login-cookie-named-step-3 = Klappe „Cookies“ auf, wähle { $site } und suche das Cookie namens { $cookie }.
login-cookie-named-step-4 = Doppelklicke auf seinen Wert und kopiere ihn.
login-cookie-named-note = Der Wert von { $cookie } allein genügt. Ein ganzer Cookie-Header aus dem Netzwerk-Tab funktioniert auch.
login-guest-title = Gastmodus
login-guest-use = Gastmodus verwenden
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
login-choose-title = Bei { $provider } anmelden
login-choose-detail = Wähle, wie du dich bei { $provider } anmeldest.

# album and playlist pages

# artist page
artist-eyebrow = Künstler
artist-monthly-listeners = { $count ->
    [one] { $value } monatlicher Hörer
   *[other] { $value } monatliche Hörer
}
artist-popular = Beliebt
artist-popular-eyebrow = Diesen Künstler entdecken
artist-popular-empty = Von diesem Künstler gibt es noch nichts zum Abspielen
artist-releases = Veröffentlichungen
artist-filter-all = Alle
artist-filter-singles = Singles
artist-filter-eps = EPs
artist-appears-on = Enthalten auf
album-also-like = Das könnte dir auch gefallen
album-tab-artists = Künstler
album-label = ℗ { $label }

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

# home page
home-quick-picks = Schnellauswahl
home-quick-picks-empty = Markiere ein paar Songs als Favoriten, dann erscheinen sie hier
home-recently-added = Kürzlich hinzugefügt
home-recently-played = Kürzlich gespielt
home-favorite-albums = Lieblingsalben
home-artists = Künstler
home-collection-albums = Alben aus deiner Sammlung

# search page
search-placeholder = Was möchtest du hören?
search-browse = Alles durchsuchen
search-submit = Suchen
search-recent = Letzte Suchen
search-forget = Aus dem Verlauf entfernen
genre-empty = Hier gibt es noch nichts zu sehen
search-best-match = Beste Übereinstimmung
search-no-matches = Keine Treffer
search-results = Ergebnisse
search-artists = Künstler
search-albums-playlists = Alben & Playlists
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } Song in der Bibliothek
       *[other] { $count } Songs in der Bibliothek
    }

# song page
song-eyebrow = Song
song-view-album = Album anzeigen
song-about = Über diesen Song
song-album = Album
song-released = Veröffentlicht
song-streams = Streams
song-position = Position
song-label = Label
song-popularity = Beliebtheit
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

# tags
tags-edit-title = Tags bearbeiten
tags-sheet-song = Titel
tags-sheet-album = Album
tags-sheet-details = Details
tags-title = Titel
tags-artist = Künstler
tags-artists = Künstler
tags-add-artist = Künstler hinzufügen
tags-remove-artist = Diesen Künstler entfernen
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
