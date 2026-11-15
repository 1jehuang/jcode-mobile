package org.jcode.mobile.nativeservicestests;

import android.os.Parcel;
import android.os.ResultReceiver;

/** Binder-only framework parcelable, without a test APK anonymous-class name. */
final class CameraResultReceiver {
    private CameraResultReceiver() {}

    static ResultReceiver forIpc(ResultReceiver receiver) {
        Parcel parcel = Parcel.obtain();
        try {
            receiver.writeToParcel(parcel, 0);
            parcel.setDataPosition(0);
            // Keep the original local receiver alive through its Binder endpoint,
            // but send a plain framework object that either UID can deserialize.
            return ResultReceiver.CREATOR.createFromParcel(parcel);
        } finally {
            parcel.recycle();
        }
    }
}
