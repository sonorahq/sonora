# login
login-restoring = Kontroluji uloženou relaci…
login-authorizing = Čekám na autorizaci v prohlížeči…
login-signed-in = Přihlášeno jako { $name }
login-failed-title = Přihlášení se nezdařilo
login-problem-region = Spotify neotevře relaci ze země, ve které právě jsi. Přihlas se ze své domovské země, nebo změň zemi na svém účtu Spotify.
login-problem-credentials = Tvoje uložená relace Spotify už není platná. Pro pokračování se přihlas znovu.
login-problem-network = Sonora se nedostala k hudební službě. Zkontroluj připojení k internetu a zkus to znovu.
login-problem-cancelled = Přihlášení nebylo schváleno a stránka v prohlížeči byla zavřena. Začni znovu, aby se přihlášení dokončilo.
login-problem-refused = Spotify přihlášení odmítla. Počkej chvíli a zkus to znovu.
login-problem-premium = Sonora streamuje přes Spotify Premium a tento účet ho nemá. Pro pokračování se přihlas účtem s Premium.
login-sign-in = Přihlásit se přes { $provider }
login-connect-cookies = Vložit cookies ručně
login-cookie-open = Otevřít { $provider }
login-cookie-submit = Pokračovat
login-cookie-hint = Sem vlož hlavičku Cookie z požadavku
login-cookie-step-1 = Otevři music.youtube.com a ujisti se, že jsi přihlášen. Nejlépe funguje anonymní okno.
login-cookie-step-2 = Stiskni F12, otevři kartu Network a obnov stránku.
login-cookie-step-3 = Vyber jakýkoli požadavek s názvem „browse“ nebo „next“.
login-cookie-step-4 = V kartě Headers najdi Cookie v části Request Headers, klikni na něj pravým tlačítkem a zkopíruj jeho hodnotu.
login-cookie-step-note = Vlož celou hodnotu včetně SAPISID a __Secure-3PAPISID.
login-cookie-header-title = Vlož své cookies { $provider } a dokonči přihlášení
login-cookie-apple-step-3 = Vyber jakýkoli požadavek na { $site }.
login-cookie-apple-note = Vlož celou hodnotu včetně { $cookie }.
login-cookie-named-title = Vlož cookie { $provider } a dokonči přihlášení
login-cookie-named-hint = Sem vlož hodnotu cookie
login-cookie-named-step-1 = Otevři { $site } a ujisti se, že jsi přihlášen.
login-cookie-named-step-2 = Stiskni F12 a ve Firefoxu otevři kartu Storage, v Chromu kartu Application.
login-cookie-named-step-3 = Rozbal Cookies, vyber { $site } a najdi cookie s názvem { $cookie }.
login-cookie-named-step-4 = Dvojklikni na jeho hodnotu a zkopíruj ji.
login-cookie-named-note = Stačí samotná hodnota { $cookie }. Funguje i celá hlavička Cookie zkopírovaná z karty Network.
login-guest-title = Režim hosta
login-guest-use = Použít režim hosta
login-usage-consent = Pomoz nám odhadnout, kolik lidí používá Sonoru.
login-device-code = Zadej tento kód na { $url }
login-server-title = Připojit se k tvému serveru Subsonic
login-server-detail = Zadej adresu libovolného serveru Subsonic nebo OpenSubsonic (Navidrome, Airsonic, Gonic, …) a pak se přihlas uživatelským jménem a heslem k serveru. Relace zůstane na tomto zařízení.
login-server-hint = https://music.example.com
login-username-hint = Uživatelské jméno
login-password-hint = Heslo
login-server-submit = Připojit
login-account-title = Vyber účet
login-account-detail = Tato relace je přihlášena k více účtům Google. Vyber ten, který má Sonora používat.
login-choose-title = Přihlásit se k { $provider }
login-choose-detail = Vyber způsob přihlášení k { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Interpret
artist-monthly-listeners = { $count ->
    [one] { $value } měsíční posluchač
    [few] { $value } měsíční posluchači
   *[other] { $value } měsíčních posluchačů
}
artist-popular = Populární
artist-popular-eyebrow = Prozkoumej tohoto interpreta
artist-popular-empty = Zatím tu není co přehrát
artist-releases = Vydání
artist-filter-all = Vše
artist-filter-singles = Singly
artist-filter-eps = EP
artist-appears-on = Objevuje se na
album-also-like = Možná se ti bude líbit
album-tab-artists = Interpreti
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profil
user-followers = { $count ->
    [one] { $value } sledující
    [few] { $value } sledující
   *[other] { $value } sledujících
}
user-following = { $count ->
    [one] { $value } sledovaný
    [few] { $value } sledovaní
   *[other] { $value } sledovaných
}
user-playlists = Veřejné playlisty
user-playlists-empty = Zatím žádné veřejné playlisty

# home page
home-quick-picks = Rychlý výběr
home-quick-picks-empty = Označ pár skladeb jako oblíbené a objeví se tady
home-recently-added = Nedávno přidané
home-recently-played = Nedávno přehrávané
home-favorite-albums = Oblíbená alba
home-artists = Interpreti
home-collection-albums = Alba z tvé knihovny

# search page
search-placeholder = Co chceš poslouchat?
search-browse = Procházet vše
search-submit = Hledat
search-recent = Nedávná hledání
search-forget = Odebrat z historie
genre-empty = Tady zatím nic není
search-best-match = Nejlepší shoda
search-no-matches = Žádné výsledky
search-results = Výsledky
search-artists = Interpreti
search-albums-playlists = Alba a playlisty
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } skladba v knihovně
        [few] { $count } skladby v knihovně
       *[other] { $count } skladeb v knihovně
    }

# song page
song-eyebrow = Skladba
song-view-album = Zobrazit album
song-about = O této skladbě
song-album = Album
song-released = Vydáno
song-streams = Přehrání
song-position = Pozice
song-label = Vydavatelství
song-popularity = Popularita
song-disc-track = Disk { $disc }, skladba { $track }
song-track = Skladba { $track }
song-credits = Autoři
song-performed-by = Účinkující
song-details = Žánry a podrobnosti
song-genres = Žánry
song-language = Jazyk
song-content = Obsah
song-explicit = Explicitní
song-clean = Bez vulgarismů
artist-about = O interpretu
artist-about-fallback = Prozkoumej populární skladby a vydání interpreta.
artist-about-open = Přejít na interpreta
song-copyright = © { $notice }

# tags
tags-edit-title = Upravit tagy
tags-sheet-song = Skladba
tags-sheet-album = Album
tags-sheet-details = Podrobnosti
tags-title = Název
tags-artist = Interpret
tags-artists = Interpreti
tags-add-artist = Přidat interpreta
tags-remove-artist = Odebrat tohoto interpreta
tags-track = Číslo skladby
tags-track-total = Celkem skladeb
tags-disc = Číslo disku
tags-disc-total = Celkem disků
tags-album = Album
tags-album-artist = Interpret alba
tags-year = Rok
tags-genre = Žánr
tags-composer = Skladatel
tags-publisher = Vydavatel
tags-isrc = ISRC
tags-comment = Komentář
