# track menu
menu-new-playlist = Yeni çalma listesi
menu-import-playlist = Çalma listesini içe aktar…
menu-no-playlists = Çalma listesi yok
menu-search-playlists = Çalma listesi ara
menu-add-to-library = Favorilere ekle
menu-remove-from-library = Favorilerden çıkar
menu-remove-tracks-from-playlist = { $count } parçayı çalma listesinden çıkar
menu-remove-tracks-from-history = { $count } parçayı geçmişten kaldır
menu-download = İndir
menu-remove-download = İndirmeyi kaldır
menu-downloading = İndiriliyor. Durdurmak için tıkla
menu-song-radio = Şarkı radyosuna git
menu-go-to-album = Albüme git
menu-go-to-artist = Sanatçıya git
menu-view-details = Ayrıntıları gör
menu-copy-link = Bağlantıyı kopyala
menu-select-all = Tümünü seç
menu-remove-from-queue = Sıradan çıkar
menu-export-playlist = Çalma listesini dışa aktar…
menu-add-playlist-to-library = Kitaplığa ekle
menu-remove-playlist-from-library = Kitaplıktan çıkar
menu-make-playlist-public = Herkese açık yap
menu-make-playlist-private = Gizli yap
menu-play-artist = Sanatçıyı çal

# playlist editor
menu-copy-link-none = Bunun paylaşılacak bir bağlantısı yok
menu-copy-link-local = Yerel sanatçıların ve çalma listelerinin bağlantısı yok
menu-save-library = Kitaplık
menu-go-to-playlist = Çalma listesine git
menu-play-next = { $count ->
    [1] Sıradaki olarak çal
   *[other] { $count } parçayı sıradaki olarak çal
}
menu-add-to-queue = { $count ->
    [1] Sıraya ekle
   *[other] { $count } parçayı sıraya ekle
}
menu-play-last = { $count ->
    [1] En son çal
   *[other] { $count } parçayı en son çal
}
menu-delete-track-files = { $count ->
    [1] Parça dosyasını sil
   *[other] { $count } parça dosyasını sil
}
menu-add-to-playlist = { $count ->
    [1] Çalma listesine ekle
    [one] { $count } parçayı çalma listesine ekle
   *[other] { $count } parçayı çalma listesine ekle
}

# smart rules
smart-rules-create-title = Yeni akıllı çalma listesi
smart-rules-edit-title = Akıllı kurallar
smart-rules-loading = Kurallar sunucudan okunuyor…
smart-rules-saving = Kaydediliyor…
smart-rules-failed-title = Kurallar okunamadı
smart-rules-match-all = Tümünü eşleştir
smart-rules-match-any = Herhangi birini eşleştir
smart-rules-match-toggle = Aşağıdaki kuralların tümünün mü, herhangi birinin mi eşleşeceğini seç
smart-rules-of-following = (aşağıdaki kurallardan)
smart-rules-add-condition = Koşul ekle
smart-rules-add-group = Grup ekle
smart-rules-remove-group = Bu grubu kaldır
smart-rules-remove-condition = Bu kuralı kaldır
smart-rules-empty-group = Henüz kural yok
smart-rules-field = Alan
smart-rules-operator = İşleç
smart-rules-value-placeholder = Değer
smart-rules-range-and = ve
smart-rules-true = Evet
smart-rules-false = Hayır
smart-rules-unknown-note = Sunucudan geldiği gibi korundu
smart-rules-synced-title = Bu çalma listesi bir dosyadan eşzamanlanıyor
smart-rules-synced-message = Sonraki tarama burada yaptığın değişikliklerin üzerine yazabilir.
smart-rules-sort-label = Sıralama ölçütü
smart-rules-sort-placeholder = title, dateloved, random…
smart-rules-order-none = Yön belirtilmemiş
smart-rules-order-asc = Artan
smart-rules-order-desc = Azalan
smart-rules-order-toggle = Sıralama yönü
smart-rules-limit-label = Sınır
smart-rules-limit-placeholder = Tüm eşleşmeler
smart-rules-problem-empty-group = Grupta kural yok
smart-rules-problem-missing-field = Koşul için alan belirtilmemiş
smart-rules-problem-range = Aralık için alt ve üst sınır gerekli
smart-rules-problem-days = Gün sayısı sayısal bir değer olmalı
smart-rules-problem-playlist = Çalma listesi kuralı için bir çalma listesi gerekli
smart-rules-problem-truth = Kural doğru veya yanlış değerini almalı
smart-rules-problem-compared = Değer kurala uygun değil
smart-rules-problem-boolean = Mantıksal alan doğru veya yanlış değerini almalı
smart-rules-problem-bad-order = Sıralama yönü asc veya desc olmalı

# playlist editor
playlist-name-placeholder = Çalma listesi adı
playlist-create-title = Çalma listesi oluştur
playlist-edit-title = Çalma listesini düzenle
playlist-cover-choose = Resim seç
playlist-cover-remove = Kapağı kaldır
playlist-cover-unopenable = Sonora bu dosyayı açamıyor
playlist-cover-too-large = Bu resim 40 MB’tan büyük
playlist-cover-not-a-picture = Bu dosya JPEG, PNG veya WebP resmi değil
playlist-delete-title = Çalma listesini sil
playlist-again-title = Yeniden eklensin mi?
playlist-again-confirm = Bu parça zaten “{ $name }” içinde. Bir kopya daha eklensin mi?
playlist-again-add = Yeniden ekle

# confirm
playlist-create-shelf = Kaydedilecek yer

# cover viewer
cover-view = Kapağı görüntüle
cover-zoom-in = Yakınlaştır
cover-zoom-out = Uzaklaştır
cover-zoom-fit = Pencereye sığdır

# confirm
confirm-delete-track-files-title = Parça dosyaları silinsin mi?
confirm-delete-track-files = Dosyalar diskten kalıcı olarak silinir. Bu işlem geri alınamaz.
confirm-uninstall-widevine-title = Widevine modülü kaldırılsın mı?
confirm-uninstall-widevine = Sonora, Google'dan indirdiği kopyayı siler. Apple Music parçaları, modül yeniden indirilene kadar çalmaz.
confirm-remove-downloads-title = Tüm indirmeler kaldırılsın mı?
confirm-remove-downloads = İndirdiğin her şarkı, her servisten, bu bilgisayardan silinir. Yeniden indirene kadar internetten çalınırlar.
confirm-remove-soloist-title = Soloist kaldırılsın mı?
confirm-remove-soloist = Sonora, Soloist uygulamasını, API anahtarını ve Soloist oturumunu siler. Spotify yeniden yerleşik oynatıcıyı kullanır.
confirm-delete-named = “{ $name }” silinsin mi? Bu işlem geri alınamaz.
