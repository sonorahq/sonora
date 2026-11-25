equalizer-advanced = Gelişmiş
equalizer-simple = Basit
equalizer-sound = Ses
equalizer-profile-default = Varsayılan
equalizer-profile-new = Profil { $number }
equalizer-profile-pasted = Yapıştırılan profil
equalizer-profile-name-hint = Profil adı
equalizer-save-as = Profil olarak kaydet
equalizer-rename = Profili yeniden adlandır
equalizer-delete = Profili sil
equalizer-bass = Bas
equalizer-treble = Tiz
equalizer-tone-reset = 0 dB'ye sıfırla
equalizer-preamp = Ön kazanç
equalizer-preamp-auto = Otomatik
equalizer-preamp-manual = Elle
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = okt
equalizer-no-gain = —
equalizer-filters = Filtreler
equalizer-no-filters = Henüz filtre yok. Bir tane ekle, yukarıdan bir hazır ayar seç ya da bir profil içe aktar.
equalizer-add-filter = Filtre ekle
equalizer-remove-filter = Filtreyi kaldır
equalizer-kind-peak = Tepe
equalizer-kind-low-shelf = Alçak raf
equalizer-kind-high-shelf = Yüksek raf
equalizer-kind-low-pass = Alçak geçiren
equalizer-kind-high-pass = Yüksek geçiren
equalizer-kind-notch = Çentik
equalizer-kind-band-pass = Bant geçiren
equalizer-kind-all-pass = Tüm geçiren
equalizer-transfer = İçe ve dışa aktar
equalizer-import = İçe aktar
equalizer-export = Dışa aktar
equalizer-import-file = Dosya içe aktar…
equalizer-export-file = Dosyaya aktar…
equalizer-imported = { $name } içe aktarıldı
equalizer-imported-skipped = { $name } içe aktarıldı, { $count ->
        [one] bir satır
       *[other] { $count } satır
    } atlandı
equalizer-import-nothing = İçe aktarılacak ekolayzer ayarı bulunamadı
equalizer-import-too-many = İçe aktarılmadı: { $count ->
        [one] bir filtre açık
       *[other] { $count } filtre açık
    }, bu bir profilin alabileceğinden fazla
equalizer-import-too-large = İçe aktarılmadı: metin 1 MB'tan büyük, bir ekolayzer profili için fazla büyük
equalizer-filter-limit =Bir profil en fazla { $max } filtre alır.
equalizer-paste-empty = Panoda metin yok
equalizer-copied = Profil Equalizer APO metni olarak kopyalandı
equalizer-exported = Profil dışa aktarıldı
equalizer-file-unreadable = Dosya okunamadı
equalizer-file-unwritable = Dosya yazılamadı
equalizer-value-refused = Önceki değer korundu
equalizer-skipped-line = Satır { $line }: { $reason }
equalizer-skipped-more = ve { $count ->
        [one] bir tane daha
       *[other] { $count } tane daha
    }
equalizer-skip-unknown-type = bilinmeyen filtre türü { $name }
equalizer-skip-unreadable = Sonora'nın okuyabildiği bir filtre değil
equalizer-skip-no-frequency = frekans yok
equalizer-skip-no-gain = kazanç yok
equalizer-skip-no-width = Q ya da bant genişliği yok
equalizer-skip-expression = ifade olarak yazılmış bir değer
equalizer-skip-no-preamp = ön kazanç bir sayı değil
equalizer-skip-unsupported = { $name } desteklenmiyor
equalizer-rules = Cihaz kuralları
equalizer-rule-other = Diğer tüm cihazlar
equalizer-rule-playing = Şimdi çalıyor
equalizer-rule-replaces = Kendi kuralının yerini alır
equalizer-rule-choose-device = Bir cihaz seç
equalizer-rule-choose-sound = Bir ses seç
equalizer-rule-add = Kural ekle
equalizer-rule-remove = Kuralı kaldır
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Frekans { $low } ile { $high } Hz arasında olmalı
equalizer-invalid-gain = Kazanç { $low } ile { $high } dB arasında olmalı
equalizer-invalid-q = Q { $low } ile { $high } arasında olmalı
equalizer-invalid-octaves = Bant genişliği { $low } ile { $high } oktav arasında olmalı
equalizer-invalid-shelf-octaves = Raf filtresi bant genişliği değil, Q alır
equalizer-invalid-preamp = Ön kazanç { $low } ile { $high } dB arasında olmalı
equalizer-overload = { $db } dB kırpılma
equalizer-overload-slight = 0,1 dB'den az kırpılma
equalizer-lower-preamp = Ön kazancı düşür
equalizer-level = Düzey
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Sonora'dan bit-perfect
equalizer-status-server = Ses sunucusu ardından bunu { $khz } kHz'e dönüştürür
equalizer-status-altered = Bit-perfect değil, nedeni:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Ses düzeyi
equalizer-status-normalized = Eşitlendi
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } kanal
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } kanal
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
equalizer-filter = Filtre { $number }
equalizer-filter-kind = Filtre { $number } türü
equalizer-mode = Mod
equalizer-preamp-level = Ön kazanç düzeyi
equalizer-no-gain-spoken = Kazanç yok
equalizer-filter-frequency = Filtre { $number } frekansı
equalizer-filter-gain = Filtre { $number } kazancı
equalizer-filter-width = Filtre { $number } genişliği
equalizer-graph = Frekans tepkisi
equalizer-graph-detail = { $count ->
    [0] Filtre yok
    [one] 1 filtre
   *[other] { $count } filtre
    }, ön kazanç { $db } dB
equalizer-rule-device = Cihaz
equalizer-status-direct = Sonora devreye girmeden Soloist tarafından çalınıyor
