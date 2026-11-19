# login
login-restoring = Duke kontrolluar seancën e ruajtur…
login-authorizing = Duke pritur autorizimin në shfletues…
login-signed-in = I lidhur si { $name }
login-failed-title = Hyrja dështoi
login-problem-region = Spotify nuk hap një seancë nga vendi ku ndodhesh. Hyr nga vendi yt i origjinës, ose ndrysho vendin në llogarinë Spotify.
login-problem-credentials = Seanca e ruajtur e Spotify nuk është më e vlefshme. Hyr përsëri për të vazhduar.
login-problem-network = Sonora nuk mund të lidhej me shërbimin muzikor. Kontrollo lidhjen e internetit dhe provo përsëri.
login-problem-cancelled = Mbyllët faqen e shfletuesit para se të miratoje hyrjen. Filloni përsëri për të përfunduar.
login-problem-refused = Spotify refuzoi hyrjen. Prit pak dhe provo përsëri.
login-problem-premium = Sonora transmeton përmes Spotify Premium, dhe kjo llogari nuk e ka. Hyr me një llogari Premium për të vazhduar.
login-sign-in = Hyr me { $provider }
login-connect-cookies = Ngjit cookies manualisht
login-cookie-open = Hap { $provider }
login-cookie-submit = Vazhdo
login-cookie-hint = Ngjit këtu header-in Cookie të kërkesës
login-cookie-step-1 = Hap music.youtube.com dhe sigurohu që je i lidhur. Një dritare inkognito funksionon më mirë.
login-cookie-step-2 = Shtyp F12, hap skedën Network dhe ringarko faqen.
login-cookie-step-3 = Zgjidh çdo kërkesë me emrin "browse" ose "next".
login-cookie-step-4 = Te Headers, gjej Cookie nën Request Headers, kliko me të djathtën dhe kopjo vlerën.
login-cookie-step-note = Sigurohu që të ngjitësh të gjithë vlerën, përfshirë SAPISID dhe __Secure-3PAPISID.
login-cookie-header-title = Ngjit cookies e { $provider } për të përfunduar hyrjen
login-cookie-apple-step-3 = Zgjidh çdo kërkesë drejt { $site }.
login-cookie-apple-note = Sigurohu që të ngjitësh të gjithë vlerën, përfshirë { $cookie }.
login-cookie-named-title = Ngjit cookie-n e { $provider } për të përfunduar hyrjen
login-cookie-named-hint = Ngjit këtu vlerën e cookie-t
login-cookie-named-step-1 = Hap { $site } dhe sigurohu që je i lidhur.
login-cookie-named-step-2 = Shtyp F12 dhe hap skedën Storage në Firefox, ose skedën Application në Chrome.
login-cookie-named-step-3 = Zgjero Cookies, zgjidh { $site } dhe gjej cookie-n me emrin { $cookie }.
login-cookie-named-step-4 = Kliko dy herë vlerën dhe kopjoje.
login-cookie-named-note = Vetëm vlera e { $cookie } mjafton. Edhe një header i plotë Cookie i kopjuar nga skeda Network funksionon.
login-guest-title = Modaliteti mysafir
login-guest-use = Përdor modalitetin mysafir
login-usage-consent = Na ndihmo të vlerësojmë sa njerëz përdorin Sonora.
login-device-code = Shkruaj këtë kod te { $url }
login-server-title = Lidhu me serverin tënd Subsonic
login-server-detail = Shkruaj adresën e çdo serveri Subsonic ose OpenSubsonic (Navidrome, Airsonic, Gonic, …), pastaj hy me emrin e përdoruesit dhe fjalëkalimin e serverit. Seanca mbetet në këtë pajisje.
login-server-hint = https://music.example.com
login-username-hint = Emri i përdoruesit
login-password-hint = Fjalëkalimi
login-server-submit = Lidhu
login-account-title = Zgjidh një llogari
login-account-detail = Kjo seancë është e lidhur me më shumë se një llogari Google. Zgjidh atë që Sonora duhet të përdorë.
login-choose-title = Hyr te { $provider }
login-choose-detail = Zgjidh se si të hysh te { $provider }.

# album and playlist pages

# artist page
artist-eyebrow = Artist
artist-monthly-listeners = { $count ->
    [one] { $value } dëgjues mujor
   *[other] { $value } dëgjues mujorë
}
artist-popular = Popullore
artist-popular-eyebrow = Eksploro këtë artist
artist-popular-empty = Ende asgjë për të luajtur nga ky artist
artist-releases = Publikime
artist-filter-all = Të gjitha
artist-filter-singles = Single
artist-filter-eps = EP
artist-appears-on = Shfaqet në
album-also-like = Mund të të pëlqejnë edhe
album-tab-artists = Artistë
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profili
user-followers = { $count ->
    [one] { $value } ndjekës
   *[other] { $value } ndjekës
}
user-following = { $count ->
   *[other] { $value } duke ndjekur
}
user-playlists = Lista publike luajtjeje
user-playlists-empty = Ende asnjë listë publike luajtjeje

# home page
home-quick-picks = Zgjedhje të shpejta
home-quick-picks-empty = Pëlqe disa këngë dhe ato do të shfaqen këtu
home-recently-added = Shtuar së fundi
home-recently-played = Luajtur së fundi
home-favorite-albums = Albumet e preferuara
home-artists = Artistë
home-collection-albums = Albume nga koleksioni yt

# search page
search-placeholder = Çfarë do të dëgjosh?
search-browse = Shfleto të gjitha
search-submit = Kërko
search-recent = Kërkimet e fundit
search-forget = Hiqe nga historiku
genre-empty = Ende asgjë për të shfaqur këtu
search-best-match = Përputhja më e mirë
search-no-matches = Asnjë përputhje
search-results = Rezultatet
search-artists = Artistë
search-albums-playlists = Albume dhe lista luajtjeje
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] { $count } këngë në Bibliotekë
       *[other] { $count } këngë në Bibliotekë
    }

# song page
song-eyebrow = Këngë
song-view-album = Shiko albumin
song-about = Rreth kësaj kënge
song-album = Albumi
song-released = Publikuar
song-streams = Transmetime
song-position = Pozicioni
song-label = Etiketa
song-popularity = Popullariteti
song-disc-track = Disku { $disc }, kënga { $track }
song-track = Kënga { $track }
song-credits = Kredite
song-performed-by = Interpretuar nga
song-details = Zhanre dhe detaje
song-genres = Zhanre
song-language = Gjuha
song-content = Përmbajtja
song-explicit = Eksplicite
song-clean = E pastër
artist-about = Rreth artistit
artist-about-fallback = Eksploro këngët dhe publikimet popullore të artistit.
artist-about-open = Shko te artisti
song-copyright = © { $notice }

# tags
tags-edit-title = Ndrysho etiketat
tags-sheet-song = Kënga
tags-sheet-album = Albumi
tags-sheet-details = Detajet
tags-title = Titulli
tags-artist = Artisti
tags-artists = Artistët
tags-add-artist = Shto një artist
tags-remove-artist = Hiq këtë artist
tags-track = Numri i këngës
tags-track-total = Këngë në publikim
tags-disc = Numri i diskut
tags-disc-total = Disket në publikim
tags-album = Albumi
tags-album-artist = Artisti i albumit
tags-year = Viti
tags-genre = Zhanri
tags-composer = Kompozitori
tags-publisher = Botuesi
tags-isrc = ISRC
tags-comment = Komenti
