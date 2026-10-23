package io.github.ccpopy.farmhelper;

import android.content.ContentValues;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.net.Uri;
import android.os.Environment;
import android.provider.MediaStore;
import java.io.IOException;

public final class WxQrImage {
    private WxQrImage() {}
    public static Uri save(Context context, byte[] bytes) throws Exception {
        BitmapFactory.Options bounds = new BitmapFactory.Options(); bounds.inJustDecodeBounds = true;
        BitmapFactory.decodeByteArray(bytes, 0, bytes.length, bounds);
        if (bounds.outWidth < 1 || bounds.outHeight < 1 || bounds.outWidth > 4096 || bounds.outHeight > 4096)
            throw new IOException("服务器未返回有效的二维码图片");
        Bitmap bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
        if (bitmap == null) throw new IOException("二维码图片解码失败");
        Uri uri = null;
        try {
            ContentValues values = new ContentValues();
            values.put(MediaStore.Images.Media.DISPLAY_NAME, "FarmHelper-WeChat-" + System.currentTimeMillis() + ".png");
            values.put(MediaStore.Images.Media.MIME_TYPE, "image/png");
            values.put(MediaStore.Images.Media.RELATIVE_PATH, Environment.DIRECTORY_PICTURES + "/FarmHelper");
            values.put(MediaStore.Images.Media.DATE_TAKEN, System.currentTimeMillis());
            values.put(MediaStore.Images.Media.IS_PENDING, 1);
            uri = context.getContentResolver().insert(MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY), values);
            if (uri == null) throw new IOException("无法创建相册图片");
            try (var output = context.getContentResolver().openOutputStream(uri)) {
                if (output == null || !bitmap.compress(Bitmap.CompressFormat.PNG, 100, output)) throw new IOException("二维码保存失败");
            }
            values.clear(); values.put(MediaStore.Images.Media.IS_PENDING, 0);
            if (context.getContentResolver().update(uri, values, null, null) != 1) throw new IOException("二维码未能发布到相册");
            return uri;
        } catch (Exception error) {
            if (uri != null) try { context.getContentResolver().delete(uri, null, null); } catch (Exception ignored) {}
            throw error;
        } finally { bitmap.recycle(); }
    }
}
