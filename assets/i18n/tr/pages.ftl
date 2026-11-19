# login
login-restoring = Kayıtlı oturumun denetleniyor…
login-authorizing = Tarayıcında izin verilmesi bekleniyor…
login-signed-in = { $name } olarak oturum açıldı
login-failed-title = Oturum açılamadı
login-problem-region = Spotify, bulunduğun ülkeden oturum açılmasına izin vermiyor. Kendi ülkenden oturum aç ya da Spotify hesabındaki ülkeyi değiştir.
login-problem-credentials = Kayıtlı Spotify oturumun artık geçerli değil. Sürdürmek için yeniden oturum aç.
login-problem-network = Sonora, müzik hizmetine ulaşamadı. İnternet bağlantını denetleyip yeniden dene.
login-problem-cancelled = Oturum açmayı onaylamadan tarayıcı sayfasını kapattın. Bitirmek için yeniden başla.
login-problem-refused = Spotify oturum açma isteğini geri çevirdi. Biraz bekleyip yeniden dene.
login-problem-premium = Sonora, Spotify Premium üzerinden yayın yapar ve bu hesapta Premium yok. Sürdürmek için Premium bir hesapla oturum aç.
login-sign-in = { $provider } ile oturum aç
login-connect-cookies = Çerezleri elle yapıştır
login-cookie-open = { $provider } sitesini aç
login-cookie-submit = Sürdür
login-cookie-hint = Cookie istek başlığını buraya yapıştır
login-cookie-step-1 = music.youtube.com adresini aç ve oturumunun açık olduğundan emin ol. En iyi sonucu gizli pencerede alırsın.
login-cookie-step-2 = F12'ye bas, Network sekmesini aç ve sayfayı yeniden yükle.
login-cookie-step-3 = "browse" ya da "next" adlı herhangi bir isteği seç.
login-cookie-step-4 = Headers içinde Request Headers altındaki Cookie'yi bul, sağ tıklayıp değerini kopyala.
login-cookie-step-note = SAPISID ve __Secure-3PAPISID dahil olmak üzere değerin tamamını yapıştırdığından emin ol.
login-cookie-header-title = Oturum açmayı bitirmek için { $provider } çerezlerini yapıştır
login-cookie-apple-step-3 = { $site } adresine giden herhangi bir isteği seç.
login-cookie-apple-note = { $cookie } dahil olmak üzere değerin tamamını yapıştırdığından emin ol.
login-cookie-named-title = Oturum açmayı bitirmek için { $provider } çerezini yapıştır
login-cookie-named-hint = Çerez değerini buraya yapıştır
login-cookie-named-step-1 = { $site } adresini aç ve oturumunun açık olduğundan emin ol.
login-cookie-named-step-2 = F12'ye bas, Firefox'ta Storage, Chrome'da Application sekmesini aç.
login-cookie-named-step-3 = Cookies bölümünü genişlet, { $site } adresini seç ve { $cookie } adlı çerezi bul.
login-cookie-named-step-4 = Değerine çift tıklayıp kopyala.
login-cookie-named-note = Yalnızca { $cookie } değeri yeterli. Network sekmesinden kopyalanan Cookie başlığının tamamı da işe yarar.
login-guest-title = Konuk modu
login-guest-use = Konuk modunu kullan
login-usage-consent = Sonora'yı kaç kişinin kullandığını tahmin etmemize yardımcı ol.
login-device-code = Bu kodu { $url } adresinde gir
login-server-title = Subsonic sunucuna bağlan
login-server-detail = Herhangi bir Subsonic ya da OpenSubsonic sunucusunun (Navidrome, Airsonic, Gonic, …) adresini gir, sonra sunucu kullanıcı adın ve parolanla oturum aç. Oturum bu cihazda kalır.
login-server-hint = https://muzik.ornek.com
login-username-hint = Kullanıcı adı
login-password-hint = Parola
login-server-submit = Bağlan
login-account-title = Bir hesap seç
login-account-detail = Bu oturumda birden çok Google hesabı açık. Sonora'nın kullanacağı hesabı seç.
login-choose-title = { $provider } oturumu aç
login-choose-detail = { $provider } için oturum açma yolunu seç.

# album and playlist pages

# artist page
artist-eyebrow = Sanatçı
artist-monthly-listeners =
    { $count ->
       *[other] { $value } aylık dinleyici
    }
artist-popular = Popüler
artist-popular-eyebrow = Bu sanatçıyı keşfet
artist-popular-empty = Bu sanatçıdan çalınacak bir şey henüz yok
artist-releases = Yayınlar
artist-filter-all = Tümü
artist-filter-singles = Tekliler
artist-filter-eps = EP'ler
artist-appears-on = Yer aldığı yayınlar
album-also-like = Bunları da sevebilirsin
album-tab-artists = Sanatçılar
album-label = ℗ { $label }

# user profile page
user-eyebrow = Profil
user-followers =
    { $count ->
       *[other] { $value } takipçi
    }
user-following =
    { $count ->
       *[other] { $value } takip ediliyor
    }
user-playlists = Herkese açık çalma listeleri
user-playlists-empty = Henüz herkese açık çalma listesi yok

# home page
home-quick-picks = Hızlı seçimler
home-quick-picks-empty = Birkaç şarkı beğen, burada görünsünler
home-recently-added = Son eklenenler
home-recently-played = Son çalınanlar
home-favorite-albums = Favori albümler
home-artists = Sanatçılar
home-collection-albums = Koleksiyonundaki albümler

# search page
search-placeholder = Ne dinlemek istersin?
search-browse = Tümüne göz at
search-submit = Ara
search-recent = Son aramalar
search-forget = Geçmişten kaldır
genre-empty = Burada henüz gösterilecek bir şey yok
search-best-match = En iyi eşleşme
search-no-matches = Eşleşme yok
search-results = Sonuçlar
search-artists = Sanatçılar
search-albums-playlists = Albümler ve çalma listeleri
search-tag = { $kind } ·
search-saved =
    { $count ->
       *[other] Kitaplıkta { $count } şarkı
    }

# song page
song-eyebrow = Şarkı
song-view-album = Albümü gör
song-about = Bu şarkı hakkında
song-album = Albüm
song-released = Yayınlanma
song-streams = Dinlenme
song-position = Sıra
song-label = Etiket
song-popularity = Popülerlik
song-disc-track = Disk { $disc }, parça { $track }
song-track = Parça { $track }
song-credits = Katkıda bulunanlar
song-performed-by = Seslendiren
song-details = Türler ve ayrıntılar
song-genres = Türler
song-language = Dil
song-content = İçerik
song-explicit = Müstehcen
song-clean = Temiz
artist-about = Sanatçı hakkında
artist-about-fallback = Sanatçının popüler şarkılarını ve yayınlarını keşfet.
artist-about-open = Sanatçıya git
song-copyright = © { $notice }

# tags
tags-edit-title = Etiketleri düzenle
tags-sheet-song = Şarkı
tags-sheet-album = Albüm
tags-sheet-details = Ayrıntılar
tags-title = Başlık
tags-artist = Sanatçı
tags-artists = Sanatçılar
tags-add-artist = Sanatçı ekle
tags-remove-artist = Bu sanatçıyı kaldır
tags-track = Parça numarası
tags-track-total = Yayındaki parça sayısı
tags-disc = Disk numarası
tags-disc-total = Yayındaki disk sayısı
tags-album = Albüm
tags-album-artist = Albüm sanatçısı
tags-year = Yıl
tags-genre = Tür
tags-composer = Besteci
tags-publisher = Yayıncı
tags-isrc = ISRC
tags-comment = Yorum
members-title = Üyeler
members-empty = Seninle birlikte kimlerin dinlediğini görmek için bir oturum başlat ya da bir oturuma katıl
login-problem-password = Sunucu bu kullanıcı adını ve parolayı kabul etmedi. Bilgilerini denetleyip yeniden dene.
login-problem-subscription = Bu Apple ID hesabında etkin abonelik yok.
login-subscription-check = Apple Music'i denetle
login-jellyfin-title = Jellyfin'e bağlan
login-jellyfin-detail = Jellyfin hesabınla oturum aç.
login-quick-connect = Quick Connect ile oturum aç
login-profile-title = Profil seç
login-profile-detail = Bu hesapta birden fazla profil var. Sonora'nın kullanacağı profili seç.
login-providers = Müzik servisleri
login-welcome-title = Sonora'ya hoş geldin
login-welcome-detail = Daha sonra Ayarlar'dan başka servisler ekleyebilirsin.
login-or = veya
artist-followers = { $count ->
    [one] { $value } takipçi
   *[other] { $value } takipçi
}
artist-favorite-songs = Favori şarkıların
artist-favorite-songs-count = { $count ->
    [one] Favorilerinde { $count } şarkı
   *[other] Favorilerinde { $count } şarkı
}
home-most-played = En çok dinlenenler
powerbar-title = Hızlı arama
powerbar-open = Aç
powerbar-song = Şarkıya git
search-open-link = Bağlantıyı Sonora'da aç
song-local = Yerel dosya
track-row-name = { $artists } – { $title }
